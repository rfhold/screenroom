use crate::{
    UserService,
    repository::{bounded, uuid},
};
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

fn live_snapshot(
    page: &screenroom_catalog_adapter::jellyfin::Page<
        screenroom_catalog_adapter::jellyfin::Progress,
    >,
) -> Result<String, Status> {
    let items: Vec<_> = page
        .items
        .iter()
        .map(|item| {
            (
                &item.external_item_id,
                item.position_seconds,
                item.runtime_seconds,
                item.played,
                &item.updated_at,
            )
        })
        .collect();
    let bytes = serde_json::to_vec(&(page.offset, page.next_offset, page.reported_total, items))
        .map_err(|_| Status::internal("progress snapshot encoding failed"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

impl UserService {
    pub async fn progress(
        &self,
        request: GetContinueWatchingRequest,
    ) -> Result<GetContinueWatchingResponse, Status> {
        crate::profiles::deadline(self.progress_inner(request)).await
    }

    async fn progress_inner(
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
        let revision = self.repository.revision(id).await?;
        let mut entries = BTreeMap::new();
        let mut snapshots = Vec::new();
        let mut links = BTreeMap::new();
        let mut pages = 0usize;
        let mut count = 0usize;
        for (source, libraries) in &scopes {
            let Some(external) = self.repository.link(id, source).await? else {
                continue;
            };
            links.insert(source.clone(), external.clone());
            {
                let provider = self
                    .connections
                    .connect(source)
                    .await
                    .map_err(|_| Status::unavailable("progress provider unavailable"))?;
                provider
                    .validate_profile(&external)
                    .await
                    .map_err(|_| Status::unavailable("progress profile unavailable"))?;
                let discovered = provider
                    .libraries()
                    .await
                    .map_err(|_| Status::unavailable("progress libraries unavailable"))?;
                let libraries: Vec<_> = discovered
                    .into_iter()
                    .filter(|library| libraries.is_empty() || libraries.contains(&library.id))
                    .map(|library| library.external_id)
                    .collect();
                let mut verification = Vec::new();
                for library in libraries {
                    let mut cursor = 0;
                    let mut total = None;
                    loop {
                        pages += 1;
                        if pages > 512 {
                            return Err(Status::resource_exhausted(
                                "progress page budget exceeded",
                            ));
                        }
                        let page = provider
                            .resume_page(&external, &library, cursor, 100)
                            .await
                            .map_err(|_| Status::unavailable("progress provider unavailable"))?;
                        if total.is_some_and(|total| total != page.reported_total) {
                            return Err(Status::unavailable("progress snapshot changed"));
                        }
                        total = Some(page.reported_total);
                        let snapshot = live_snapshot(&page)?;
                        snapshots.push((source.clone(), external.clone(), snapshot.clone()));
                        verification.push((library.clone(), cursor, snapshot));
                        count += page.items.len();
                        if count > 10_000 {
                            return Err(Status::resource_exhausted(
                                "progress entry budget exceeded",
                            ));
                        }
                        for item in page.items {
                            if item.played {
                                continue;
                            }
                            let updated = item
                                .updated_at
                                .ok_or_else(|| Status::unavailable("invalid progress timestamp"))?;
                            let timestamp =
                                screenroom_backend::chrono::DateTime::parse_from_rfc3339(&updated)
                                    .map_err(|_| {
                                        Status::unavailable("invalid progress timestamp")
                                    })?;
                            let duration = item.runtime_seconds.unwrap_or(0);
                            let entry = ContinueWatchingEntry {
                                source_id: source.clone(),
                                external_item_id: item.external_item_id,
                                position_seconds: item.position_seconds,
                                duration_seconds: duration,
                                progress_percent: if duration == 0 {
                                    0
                                } else {
                                    ((u128::from(item.position_seconds) * 100
                                        / u128::from(duration))
                                    .min(100)) as u32
                                },
                                updated_at: timestamp.to_utc().to_rfc3339(),
                            };
                            let key = (source.clone(), entry.external_item_id.clone());
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
                        match page.next_offset {
                            Some(next) if next > cursor => cursor = next,
                            Some(_) => return Err(Status::unavailable("progress cursor loop")),
                            None => break,
                        }
                    }
                }
                // Offset pages have no authoritative revision; reject an observed changing traversal.
                for (library, cursor, snapshot) in verification {
                    pages += 1;
                    if pages > 512 {
                        return Err(Status::resource_exhausted("progress page budget exceeded"));
                    }
                    let page = provider
                        .resume_page(&external, &library, cursor, 100)
                        .await
                        .map_err(|_| Status::unavailable("progress provider unavailable"))?;
                    count += page.items.len();
                    if count > 10_000 {
                        return Err(Status::resource_exhausted("progress entry budget exceeded"));
                    }
                    if live_snapshot(&page)? != snapshot {
                        return Err(Status::unavailable("progress snapshot changed"));
                    }
                }
                continue;
            }
        }
        self.repository.user(id).await?;
        for source in scopes.keys() {
            if self.repository.link(id, source).await?.as_ref() != links.get(source) {
                return Err(Status::aborted("profile link changed during progress read"));
            }
        }
        if self.repository.revision(id).await? != revision {
            return Err(Status::aborted("profile link changed during progress read"));
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
        let binding = serde_json::to_vec(&(
            id.to_string(),
            revision,
            scopes,
            size,
            links,
            snapshots,
            state,
        ))
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
            source_id: "owned-source".into(),
            library_id: library.into(),
        }
    }
    #[test]
    fn normalized_scopes_are_deterministic_and_all_dominates() {
        assert_eq!(
            normalize(&[scope("movies"), scope("tv"), scope("movies")]).unwrap(),
            normalize(&[scope("tv"), scope("movies")]).unwrap()
        );
        assert!(normalize(&[scope("movies"), scope("")]).unwrap()["owned-source"].is_empty());
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
