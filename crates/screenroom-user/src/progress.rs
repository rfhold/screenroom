use crate::{
    UserService,
    repository::{bounded, uuid},
};
use screenroom_catalog_adapter::{Coverage, JELLYFIN_SOURCE, MOVIES_LIBRARY, TV_LIBRARY};
use screenroom_proto::{common::v1::SourceLibraryScope, user::internal::v1::*};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use tonic::Status;

fn normalize(scopes: &[SourceLibraryScope]) -> Result<BTreeMap<String, BTreeSet<String>>, Status> {
    if scopes.is_empty() || scopes.len() > 32 {
        return Err(Status::invalid_argument("expected 1..32 scopes"));
    }
    let mut result: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for scope in scopes {
        bounded(&scope.source_id)?;
        if !scope.library_id.is_empty() {
            bounded(&scope.library_id)?;
        }
        let libraries = result.entry(scope.source_id.clone()).or_default();
        libraries.insert(scope.library_id.clone());
    }
    for libraries in result.values_mut() {
        if libraries.contains("") {
            libraries.clear();
        }
    }
    Ok(result)
}

fn token(offset: usize, binding: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(binding);
    hash.update(offset.to_be_bytes());
    format!("{offset}:{:x}", hash.finalize())
}

fn offset(cursor: &str, binding: &[u8], size: usize, total: usize) -> Result<usize, Status> {
    if cursor.is_empty() {
        return Ok(0);
    }
    if cursor.len() > 100 {
        return Err(Status::invalid_argument("invalid page token"));
    }
    let offset = cursor
        .split_once(':')
        .and_then(|(n, _)| n.parse::<usize>().ok())
        .ok_or_else(|| Status::invalid_argument("invalid page token"))?;
    if offset == 0 || offset >= total || offset % size != 0 || cursor != token(offset, binding) {
        return Err(Status::invalid_argument("invalid or stale page token"));
    }
    Ok(offset)
}

impl UserService {
    pub async fn progress(
        &self,
        request: GetContinueWatchingRequest,
    ) -> Result<GetContinueWatchingResponse, Status> {
        let id = uuid(&request.user_id)?;
        let scopes = normalize(&request.scopes)?;
        let size = match request.page_size {
            0 => 50,
            1..=200 => request.page_size as usize,
            _ => return Err(Status::invalid_argument("page size exceeds 200")),
        };
        self.repository.user(id).await?;
        let mut entries = BTreeMap::new();
        let mut snapshots = Vec::new();
        for (source, libraries) in &scopes {
            let Some(external) = self.repository.link(id, source).await? else {
                continue;
            };
            if source != JELLYFIN_SOURCE {
                return Err(Status::failed_precondition(
                    "linked source has no progress adapter",
                ));
            }
            let libraries: Vec<_> = if libraries.is_empty() {
                vec![MOVIES_LIBRARY.into(), TV_LIBRARY.into()]
            } else {
                libraries.iter().cloned().collect()
            };
            let mut cursor = None;
            let mut seen = BTreeSet::new();
            let mut complete = false;
            for _ in 0..16 {
                let page = self
                    .adapters
                    .continue_watching(&external, &libraries, 100, cursor.as_deref())
                    .map_err(|error| match error {
                        screenroom_catalog_adapter::Error::InvalidScope => {
                            Status::invalid_argument("unsupported progress library")
                        }
                        _ => Status::unavailable("progress provider unavailable"),
                    })?;
                if page.coverage == Coverage::Partial {
                    return Err(Status::unavailable("incomplete progress provider response"));
                }
                snapshots.push((source.clone(), external.clone(), page.evidence.snapshot));
                for item in page.items {
                    if item.reference.source != *source || item.reference.external_id.is_empty() {
                        return Err(Status::unavailable(
                            "incompatible progress provider reference",
                        ));
                    }
                    let timestamp =
                        screenroom_backend::chrono::DateTime::parse_from_rfc3339(&item.updated_at)
                            .map_err(|_| {
                                Status::unavailable("invalid progress provider timestamp")
                            })?;
                    let entry = ContinueWatchingEntry {
                        source_id: source.clone(),
                        external_item_id: item.reference.external_id,
                        position_seconds: item.position_seconds,
                        duration_seconds: item.runtime_seconds,
                        progress_percent: if item.runtime_seconds == 0 {
                            0
                        } else {
                            ((u128::from(item.position_seconds) * 100
                                / u128::from(item.runtime_seconds))
                            .min(100)) as u32
                        },
                        updated_at: timestamp.to_utc().to_rfc3339(),
                    };
                    let key = (entry.source_id.clone(), entry.external_item_id.clone());
                    if let Some((previous, _)) = entries.get(&key) {
                        if previous != &entry {
                            return Err(Status::unavailable(
                                "conflicting progress provider duplicate",
                            ));
                        }
                    } else {
                        entries.insert(key, (entry, timestamp));
                    }
                }
                match page.next_cursor {
                    Some(next) if seen.insert(next.clone()) => cursor = Some(next),
                    Some(_) => return Err(Status::unavailable("progress provider cursor loop")),
                    None => {
                        complete = true;
                        break;
                    }
                }
            }
            if !complete {
                return Err(Status::resource_exhausted(
                    "progress provider page budget exceeded",
                ));
            }
        }
        let mut entries: Vec<_> = entries.into_values().collect();
        entries.sort_by(|(a, ta), (b, tb)| {
            tb.cmp(ta)
                .then_with(|| a.source_id.cmp(&b.source_id))
                .then_with(|| a.external_item_id.cmp(&b.external_item_id))
        });
        let entries: Vec<_> = entries.into_iter().map(|(entry, _)| entry).collect();
        let state: Vec<_> = entries
            .iter()
            .map(|e| {
                (
                    &e.source_id,
                    &e.external_item_id,
                    e.position_seconds,
                    e.duration_seconds,
                    e.progress_percent,
                    &e.updated_at,
                )
            })
            .collect();
        let binding = serde_json::to_vec(&(id.to_string(), scopes, size, snapshots, state))
            .map_err(|_| Status::internal("progress snapshot encoding failed"))?;
        let start = offset(&request.page_token, &binding, size, entries.len())?;
        let end = (start + size).min(entries.len());
        Ok(GetContinueWatchingResponse {
            entries: entries[start..end].to_vec(),
            next_page_token: if end < entries.len() {
                token(end, &binding)
            } else {
                String::new()
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope(library: &str) -> SourceLibraryScope {
        SourceLibraryScope {
            source_id: JELLYFIN_SOURCE.into(),
            library_id: library.into(),
        }
    }
    #[test]
    fn normalized_scopes_are_deterministic_and_all_dominates() {
        assert_eq!(
            normalize(&[scope("movies"), scope("tv"), scope("movies")]).unwrap(),
            normalize(&[scope("tv"), scope("movies")]).unwrap()
        );
        assert!(normalize(&[scope("movies"), scope("")]).unwrap()[JELLYFIN_SOURCE].is_empty());
        assert!(normalize(&[]).is_err());
        assert!(normalize(&vec![scope("tv"); 33]).is_err());
    }
    #[test]
    fn cursor_binds_user_scope_snapshot_size_and_offset() {
        let cursor = token(2, b"user/scopes/snapshot/size");
        assert_eq!(
            offset(&cursor, b"user/scopes/snapshot/size", 2, 4).unwrap(),
            2
        );
        assert!(offset(&cursor, b"another-user", 2, 4).is_err());
        assert!(offset(&cursor, b"user/scopes/snapshot/size", 3, 4).is_err());
        assert!(offset("99999999999999999999999999:no", b"", 2, 4).is_err());
        assert!(offset(&cursor, b"user/scopes/snapshot/size", 2, 2).is_err());
    }
}
