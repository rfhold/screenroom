use screenroom_catalog_adapter::{
    MediaKind,
    jellyfin::{ApiKey, ConnectionConfig, Item, LiveJellyfin},
};
use screenroom_index_worker::{live::map_item, mapping};
use std::collections::BTreeMap;

#[test]
fn real_media_kinds_are_explicit() {
    for (kind, name) in [
        (MediaKind::Movie, "movie"),
        (MediaKind::Series, "series"),
        (MediaKind::Season, "season"),
        (MediaKind::Episode, "episode"),
    ] {
        assert_eq!(mapping::kind(kind), name);
    }
}
#[tokio::test]
async fn jellyfin_mapping_keeps_provider_identity_hierarchy_and_document_without_enrichment() {
    let adapter = LiveJellyfin::from_api_key(
        ConnectionConfig {
            source_key: "owned".into(),
            base_url: "http://127.0.0.1:1".into(),
            timeout_ms: 1000,
            max_response_bytes: 4096,
        },
        ApiKey::new("test-only".into()).unwrap(),
    )
    .unwrap();
    let item = Item {
        id: adapter.public_id("episode"),
        external_id: "episode".into(),
        parent_external_id: Some("season".into()),
        kind: MediaKind::Episode,
        title: "Owned episode".into(),
        overview: Some("Source supplied overview".into()),
        year: Some(2026),
        index: Some(1),
        provider_ids: BTreeMap::from([("imdb".into(), "tt123".into())]),
        artwork: vec![],
        runtime_seconds: Some(120),
    };
    let write = map_item(&adapter, &item, "library").unwrap();
    assert_eq!(write.external_id, "episode");
    assert_eq!(write.parent_public_id, Some(adapter.public_id("season")));
    assert_eq!(write.external_ids[0].external_id, "tt123");
    assert!(write.document.contains("Source supplied overview"));
    assert!(write.credits.is_empty());
    assert!(write.metadata["metadata_rights"].is_null());
    assert_eq!(write.provenance["provider"], "jellyfin");
}
