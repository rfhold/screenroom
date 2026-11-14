use super::*;
use crate::catalog_delta::{Capability, Checkpoint, FallbackReason, ValidationError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}
impl ServerVersion {
    pub fn parse(value: &str) -> Option<Self> {
        let parts: Vec<_> = value.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        let parse = |s: &str| {
            if s.is_empty()
                || s.len() > 5
                || !s.bytes().all(|b| b.is_ascii_digit())
                || (s.len() > 1 && s.starts_with('0'))
            {
                return None;
            }
            s.parse::<u32>().ok()
        };
        Some(Self {
            major: parse(parts[0])?,
            minor: parse(parts[1])?,
            patch: parse(parts[2])?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCapabilities {
    pub version: Option<ServerVersion>,
    pub catalog_delta: Capability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaError {
    Provider(Error),
    FullScanRequired(FallbackReason),
    Validation(ValidationError),
}
impl std::fmt::Display for DeltaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for DeltaError {}
impl From<Error> for DeltaError {
    fn from(error: Error) -> Self {
        Self::Provider(error)
    }
}
impl From<ValidationError> for DeltaError {
    fn from(error: ValidationError) -> Self {
        Self::Validation(error)
    }
}

/// Exact normalized metadata equality, including artwork tags and ancestry, but
/// excluding personal state and transport URLs. Not a provider ETag or a timestamp.
#[derive(Clone, PartialEq, Eq)]
pub struct MetadataRevision(Vec<u8>);
impl MetadataRevision {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}
impl std::fmt::Debug for MetadataRevision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MetadataRevision([REDACTED])")
    }
}

#[derive(Debug, Clone)]
pub struct DeltaItem {
    pub item: Item,
    pub revision: MetadataRevision,
}
#[derive(Debug, Clone)]
pub struct DeltaPage {
    pub page: Page<DeltaItem>,
    /// Resolve these ancestors within the selected library before persisting children.
    pub parent_external_ids: Vec<String>,
    /// Changed series/seasons can affect descendants without advancing their saved
    /// timestamps. Queue bounded child refreshes; this is not a complete child list.
    pub refresh_children_external_ids: Vec<String>,
}

impl LiveJellyfin {
    /// 10.10 has a known MinDateLastSaved SQL parameter mismatch. Only the stable
    /// 10.11 release line with the corrected repository query is admitted; unknown
    /// release lines and prereleases fail closed. This is not deletion capability.
    pub async fn catalog_delta_capabilities(
        &self,
    ) -> std::result::Result<ServerCapabilities, DeltaError> {
        #[derive(Deserialize)]
        struct Info {
            #[serde(rename = "Version")]
            version: Option<serde_json::Value>,
        }
        let info: Info = self.json(&["System", "Info", "Public"], &[]).await?;
        let version = info
            .version
            .as_ref()
            .and_then(|v| v.as_str())
            .and_then(ServerVersion::parse);
        let catalog_delta = match version {
            Some(ServerVersion {
                major: 10,
                minor: 11,
                ..
            }) => Capability::TimestampLowerBound,
            Some(_) => Capability::FullScanRequired(FallbackReason::UnsupportedVersion),
            None => Capability::FullScanRequired(FallbackReason::InvalidVersion),
        };
        Ok(ServerCapabilities {
            version,
            catalog_delta,
        })
    }

    /// Mutable offset traversal with supported deterministic sort fields, never a
    /// snapshot or a guaranteed total order (the API has no Id sort). Ties and
    /// concurrent changes can omit rows even when duplicate/total guards pass. A page
    /// failure leaves the checkpoint unchanged. Capability is rechecked per page
    /// so caller-created or cross-provider capability tokens cannot bypass gating.
    /// Filter errors are explicit fallback, never converted into an empty page.
    pub async fn library_delta_page(
        &self,
        library: &Library,
        checkpoint: &mut Checkpoint,
    ) -> std::result::Result<DeltaPage, DeltaError> {
        let query = checkpoint.query();
        if query.scope().source_key() != self.source_key()
            || query.scope().library_id() != library.external_id
        {
            return Err(ValidationError::Scope.into());
        }
        if !matches!(library.kind, LibraryType::Movies | LibraryType::Television) {
            return Err(DeltaError::FullScanRequired(
                FallbackReason::UnsupportedLibrary,
            ));
        }
        let offset = checkpoint.request_offset()?;
        let limit = query.page_size();
        if let Capability::FullScanRequired(reason) =
            self.catalog_delta_capabilities().await?.catalog_delta
        {
            return Err(DeltaError::FullScanRequired(reason));
        }
        let mut params = page_query(&library.external_id, offset, limit)?;
        for (key, value) in &mut params {
            if *key == "SortBy" {
                *value = "SortName,DateCreated".into();
            }
            if *key == "IncludeItemTypes" && library.kind == LibraryType::Movies {
                *value = "Movie".into();
            }
            if *key == "IncludeItemTypes" && library.kind == LibraryType::Television {
                *value = "Series,Season,Episode".into();
            }
        }
        params.extend([
            (
                "MinDateLastSaved",
                query.window().lower_bound().as_str().into(),
            ),
            ("CollapseBoxSetItems", "false".into()),
            ("EnableUserData", "false".into()),
        ]);
        let rows: ItemsDto = match self.json(&["Items"], &params).await {
            Err(Error::Http(400 | 405 | 422 | 501)) | Err(Error::NotFound) => {
                return Err(DeltaError::FullScanRequired(FallbackReason::FilterRejected));
            }
            other => other?,
        };
        let (next_offset, reported_total) = rows.bounds(offset, limit)?;
        let mut items = Vec::new();
        let mut parents = BTreeSet::new();
        let mut children = BTreeSet::new();
        for dto in rows.items {
            if dto
                .parent_id
                .as_ref()
                .is_some_and(|parent| parent == &dto.id)
                || (matches!(dto.kind.as_str(), "Season" | "Episode")
                    && dto.parent_id.as_deref() == Some(library.external_id.as_str()))
            {
                return Err(Error::Hierarchy.into());
            }
            let item = self.item(dto)?;
            if (library.kind == LibraryType::Movies && item.kind != MediaKind::Movie)
                || (library.kind == LibraryType::Television && item.kind == MediaKind::Movie)
            {
                return Err(Error::InvalidResponse.into());
            }
            if let Some(parent) = &item.parent_external_id {
                parents.insert(parent.clone());
            }
            if matches!(item.kind, MediaKind::Series | MediaKind::Season) {
                children.insert(item.external_id.clone());
            }
            let revision = revision(&item)?;
            items.push(DeltaItem { item, revision });
        }
        checkpoint.accept(
            &items
                .iter()
                .map(|i| i.item.external_id.clone())
                .collect::<Vec<_>>(),
            next_offset,
            reported_total,
        )?;
        Ok(DeltaPage {
            page: Page {
                items,
                offset,
                next_offset,
                reported_total,
                authoritative: false,
            },
            parent_external_ids: parents.into_iter().collect(),
            refresh_children_external_ids: children.into_iter().collect(),
        })
    }
}

fn revision(item: &Item) -> Result<MetadataRevision> {
    let artwork: Vec<_> = item
        .artwork
        .iter()
        .map(|a| (&a.item_id, &a.kind, &a.tag))
        .collect();
    serde_json::to_vec(&serde_json::json!({
        "version": 1, "id": item.external_id, "parent": item.parent_external_id,
        "kind": item.kind, "title": item.title, "overview": item.overview,
        "year": item.year, "index": item.index, "providers": item.provider_ids,
        "artwork": artwork, "runtime_seconds": item.runtime_seconds,
    }))
    .map(MetadataRevision)
    .map_err(|_| Error::InvalidResponse)
}

#[cfg(test)]
mod tests;
