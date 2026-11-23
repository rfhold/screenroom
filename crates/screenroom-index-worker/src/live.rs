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
        "provider_ids":item.provider_ids,"provider_references":item.provider_ids.iter().map(|(provider,id)|json!({"provider":provider,"status":"source_supplied","url":reference_url(provider,id,item.kind).unwrap_or_default()})).collect::<Vec<_>>(),"credits":[],"tags":[],"artwork_keys":item.artwork.iter().map(artwork_key).collect::<Vec<_>>(),
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
fn reference_url(
    provider: &str,
    id: &str,
    kind: screenroom_catalog_adapter::MediaKind,
) -> Option<String> {
    use screenroom_catalog_adapter::MediaKind;
    let digits = |value: &str| {
        !value.is_empty()
            && value.len() <= 20
            && value.bytes().all(|b| b.is_ascii_digit())
            && value.bytes().any(|b| b != b'0')
    };
    if provider.eq_ignore_ascii_case("imdb") && id.strip_prefix("tt").is_some_and(digits) {
        return Some(format!("https://www.imdb.com/title/{id}/"));
    }
    if provider.eq_ignore_ascii_case("tmdb") && digits(id) {
        let entity = match kind {
            MediaKind::Movie => "movie",
            MediaKind::Series => "tv",
            // Episode/season IDs are not series IDs, and no series TMDB context is supplied.
            MediaKind::Episode | MediaKind::Season => return None,
        };
        return Some(format!("https://www.themoviedb.org/{entity}/{id}"));
    }
    None
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

#[cfg(test)]
mod reference_tests {
    use super::*;
    use screenroom_catalog_adapter::MediaKind;

    #[test]
    fn mapping_preserves_ids_and_provenance_with_and_without_links() {
        use screenroom_catalog_adapter::jellyfin::{ApiKey, ConnectionConfig};
        let adapter = LiveJellyfin::from_api_key(
            ConnectionConfig {
                source_key: "isolated".into(),
                base_url: "http://127.0.0.1:1/".into(),
                timeout_ms: 1000,
                max_response_bytes: 1024,
            },
            ApiKey::new("isolated-test-key".into()).unwrap(),
        )
        .unwrap();
        let mut item = Item {
            id: "public".into(),
            external_id: "private".into(),
            parent_external_id: None,
            kind: MediaKind::Movie,
            title: "Test".into(),
            overview: None,
            year: None,
            index: None,
            provider_ids: [
                ("Imdb".into(), "tt1234567".into()),
                ("Tmdb".into(), "123".into()),
                ("Unknown".into(), "opaque".into()),
                ("Malformed".into(), "123/secret".into()),
            ]
            .into(),
            artwork: vec![],
            runtime_seconds: None,
        };
        for kind in [MediaKind::Movie, MediaKind::Episode] {
            item.kind = kind;
            let mapped = map_item(&adapter, &item, "library").unwrap();
            assert_eq!(mapped.metadata["provider_ids"], json!(item.provider_ids));
            assert_eq!(mapped.provenance["provider"], "jellyfin");
            assert_eq!(mapped.external_ids.len(), item.provider_ids.len());
            for reference in mapped.metadata["provider_references"].as_array().unwrap() {
                assert_eq!(reference["status"], "source_supplied");
                let provider = reference["provider"].as_str().unwrap();
                let original = &item.provider_ids[provider];
                assert!(mapped.external_ids.iter().any(|id| id.provider == provider
                    && id.external_id == *original
                    && id.evidence["provider"] == "jellyfin"));
                assert_eq!(
                    reference["url"],
                    reference_url(provider, original, kind).unwrap_or_default()
                );
            }
        }
    }

    #[test]
    fn known_references_use_kind_aware_validated_ids() {
        for kind in [MediaKind::Movie, MediaKind::Series, MediaKind::Episode] {
            assert_eq!(
                reference_url("IMDb", "tt0123456", kind).as_deref(),
                Some("https://www.imdb.com/title/tt0123456/")
            );
        }
        assert_eq!(
            reference_url("Tmdb", "123", MediaKind::Movie).as_deref(),
            Some("https://www.themoviedb.org/movie/123")
        );
        assert_eq!(
            reference_url("tmdb", "123", MediaKind::Series).as_deref(),
            Some("https://www.themoviedb.org/tv/123")
        );
        for kind in [MediaKind::Episode, MediaKind::Season] {
            assert!(reference_url("Tmdb", "123", kind).is_none());
        }
        for id in [
            "",
            "0",
            "-1",
            "123/456",
            "123?token=secret",
            "１２３",
            " 123",
            "123 ",
            "123456789012345678901",
        ] {
            assert!(
                reference_url("Tmdb", id, MediaKind::Movie).is_none(),
                "{id}"
            );
            assert!(
                reference_url("Imdb", &format!("tt{id}"), MediaKind::Movie).is_none(),
                "{id}"
            );
        }
        for id in ["123", "nm123", "TT123", "https://imdb.com/title/tt123"] {
            assert!(reference_url("Imdb", id, MediaKind::Movie).is_none());
        }
        assert!(reference_url("unknown", "123", MediaKind::Movie).is_none());
    }
}
