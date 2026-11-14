//! Real Jellyfin mapping and private provider connections.
use crate::{Worker, mapping};
use screenroom_backend::{
    Error, Result,
    catalog::{ExternalIdWrite, ItemWrite},
    embedding::{DOCUMENT_VERSION, MAX_INPUT_CHARS, input_hash, preprocess},
    ingest::Claim,
    media::{
        ArtworkInputManifest, ArtworkInputWrite, ArtworkReferenceInput, ArtworkVariant,
        ArtworkVariantInput, ValidatedImage,
    },
};
use screenroom_catalog_adapter::jellyfin::{Artwork, Item, LiveJellyfin};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

impl Worker {
    pub fn with_connections(
        mut self,
        connections: screenroom_backend::connections::ConnectionRepository,
    ) -> Self {
        self.connections = connections;
        self
    }
    pub(crate) async fn provider(
        &self,
        claim: &Claim,
        source: &str,
    ) -> Result<Option<LiveJellyfin>> {
        use screenroom_backend::ingest::{
            OperationClass, RequestErrorClass, RequestOutcome, RequestOutcomeWrite,
        };
        let mut resolved = self.connections.resolve(source).await?;
        resolved.provider = self.transport(claim).provider(resolved.provider);
        let receipt = self
            .ingest
            .reserve_work_request(claim, OperationClass::Discovery)
            .await?;
        let result = resolved.provider.server_identity().await;
        if let Err(screenroom_catalog_adapter::jellyfin::Error::RateLimited {
            retry_after_seconds,
        }) = &result
        {
            self.ingest
                .defer_work(
                    claim,
                    &RequestOutcomeWrite {
                        receipt_id: receipt.id,
                        outcome: RequestOutcome::Retryable,
                        error_class: Some(RequestErrorClass::Throttled),
                    },
                    retry_after_seconds.unwrap_or(60).clamp(1, 86400) as i32,
                )
                .await?;
            return Ok(None);
        }
        let failure = match result {
            Ok(identity) if resolved.validate_identity(&identity).is_ok() => None,
            Ok(_) => Some((RequestErrorClass::Invalid, false)),
            Err(error) => Some(crate::full::provider_failure(&error)),
        };
        self.ingest
            .record_work_outcome(
                claim,
                &RequestOutcomeWrite {
                    receipt_id: receipt.id,
                    outcome: match failure {
                        None => RequestOutcome::Success,
                        Some((_, true)) => RequestOutcome::Retryable,
                        Some((_, false)) => RequestOutcome::Permanent,
                    },
                    error_class: failure.map(|(class, _)| class),
                },
            )
            .await?;
        if let Some((_, retryable)) = failure {
            self.ingest
                .fail(
                    claim,
                    if retryable {
                        screenroom_backend::queue::FailureClass::Transient
                    } else {
                        screenroom_backend::queue::FailureClass::Permanent
                    },
                    "connection_identity",
                    None,
                )
                .await?;
            return Ok(None);
        }
        Ok(Some(resolved.provider))
    }
}

pub fn map_item(adapter: &LiveJellyfin, item: &Item, library: &str) -> Result<ItemWrite> {
    let metadata = json!({"id":item.id,"kind":mapping::kind(item.kind),"title":item.title,"subtitle":"",
        "overview":item.overview,"year":item.year,"index":item.index,"runtime":item.runtime_seconds.map(|seconds|json!({"seconds":seconds})),
        "parent_id":item.parent_external_id.as_deref().map(|p|adapter.public_id(p)),
        "provider_ids":item.provider_ids,"provider_references":item.provider_ids.keys().map(|provider|json!({"provider":provider,"status":"source_supplied","url":""})).collect::<Vec<_>>(),"credits":[],"tags":[],"artwork_keys":item.artwork.iter().map(artwork_key).collect::<Vec<_>>(),
        "artwork_availability":if item.artwork.is_empty(){"missing"}else{"pending"},
        "title_rights":null,"metadata_rights":null,"metadata_retrieved_at":null,"rating":null});
    let document = preprocess(&format!(
        "{} {}",
        item.title,
        item.overview.as_deref().unwrap_or("")
    ))
    .chars()
    .take(MAX_INPUT_CHARS)
    .collect::<String>();
    Ok(ItemWrite {
        public_id: item.id.clone(),
        kind: mapping::kind(item.kind).into(),
        parent_public_id: item
            .parent_external_id
            .as_deref()
            .map(|p| adapter.public_id(p)),
        external_id: item.external_id.clone(),
        source_revision: None,
        content_hash: input_hash(&metadata.to_string()).to_vec(),
        metadata,
        provenance: json!({"provider":"jellyfin","license":null,"retrieved_at":null}),
        library_external_ids: vec![library.into()],
        external_ids: item
            .provider_ids
            .iter()
            .map(|(provider, id)| ExternalIdWrite {
                provider: provider.clone(),
                entity_kind: mapping::kind(item.kind).into(),
                external_id: id.clone(),
                evidence: json!({"provider":"jellyfin"}),
            })
            .collect(),
        credits: vec![],
        document_title: item.title.clone(),
        document,
        document_version: DOCUMENT_VERSION,
    })
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(crate) fn artwork_key(art: &Artwork) -> String {
    format!(
        "live:{}",
        hex(&input_hash(&format!("{}:{}", art.item_id, art.kind)))
    )
}
pub(crate) fn valid_artwork(art: &Artwork) -> bool {
    [&art.item_id, &art.kind, &art.tag].iter().all(|s| {
        !s.is_empty()
            && s.len() <= 200
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    })
}
pub(crate) fn parse_artwork(rights: &Value) -> Result<Artwork> {
    let get = |key| {
        rights
            .get("provider_input")
            .and_then(|v| v.get(key))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or(Error::Invalid("artwork input"))
    };
    let art = Artwork {
        item_id: get("item_id")?,
        kind: get("kind")?,
        tag: get("tag")?,
    };
    if !valid_artwork(&art) {
        return Err(Error::Invalid("artwork input"));
    }
    Ok(art)
}
pub(crate) fn artwork_input(art: &Artwork, image: &ValidatedImage) -> ArtworkInputWrite {
    let digest: [u8; 32] = Sha256::digest(image.bytes()).into();
    let kind = match art.kind.as_str() {
        "Primary" => "poster",
        "Thumb" => "thumbnail",
        "Logo" => "logo",
        _ => "backdrop",
    };
    ArtworkInputWrite {
        reference: ArtworkReferenceInput {
            source_artwork_id: artwork_key(art),
            kind: kind.into(),
            rights: json!({"license":null,"provider":"jellyfin","provider_input":{"item_id":art.item_id,"kind":art.kind,"tag":art.tag}}),
            item_external_id: Some(art.item_id.clone()),
            person_external_id: None,
            role: kind.into(),
            ordinal: 0,
        },
        manifest: ArtworkInputManifest {
            version: 1,
            variants: vec![
                ArtworkVariantInput {
                    variant: ArtworkVariant::Original,
                    sha256: digest,
                },
                ArtworkVariantInput {
                    variant: ArtworkVariant::Native,
                    sha256: digest,
                },
            ],
        },
    }
}
