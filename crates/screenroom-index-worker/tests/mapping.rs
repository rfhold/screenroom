use screenroom_backend::embedding::{MAX_INPUT_CHARS, input_hash, preprocess};
use screenroom_catalog_adapter::{Faults, MockAdapters};
use screenroom_index_worker::mapping;
use sha2::{Digest, Sha256};

fn adapters() -> MockAdapters {
    MockAdapters::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/catalog"),
        Faults::default(),
    )
    .unwrap()
}

#[test]
fn complete_metadata_and_bounded_documents() {
    let a = adapters();
    let mut entry = a.jellyfin_item("big-buck-bunny").unwrap();
    entry.item.overview = "interesting story ".repeat(2000);
    let item = mapping::item(&entry, "snapshot", true).unwrap();
    assert_eq!(item.metadata, serde_json::to_value(&entry.item).unwrap());
    assert!(item.metadata.get("progress_percent").is_none());
    assert_eq!(item.document.chars().count(), MAX_INPUT_CHARS);
    assert_eq!(
        input_hash(&item.document),
        input_hash(&preprocess(&item.document))
    );
    assert_eq!(item.parent_public_id, None);
    assert!(!item.external_ids.is_empty());
    assert_eq!(item.credits.len(), entry.item.credits.len());
}

#[test]
fn associations_exist_before_variant_jobs_and_preserve_rights() {
    let a = adapters();
    let entry = a.jellyfin_item("his-girl-friday").unwrap();
    let (refs, omissions) = mapping::artwork(&a, &entry, true).unwrap();
    assert!(!refs.is_empty());
    assert!(omissions.is_empty());
    for r in refs {
        assert!(r.reference.item_external_id.is_some() || r.reference.person_external_id.is_some());
        assert!(r.reference.rights.get("licenseUrl").is_some());
        assert_eq!(r.manifest.variants.len(), 2);
        assert!(r.fingerprint().is_ok());
        for variant in &r.manifest.variants {
            let image = mapping::artwork_image(&a, &r.reference.source_artwork_id, variant.variant)
                .unwrap();
            let expected: [u8; 32] = Sha256::digest(image.bytes()).into();
            assert_eq!(variant.sha256, expected);
        }
    }
}

#[test]
fn branding_preserves_svg_original_and_raster_native_bytes() {
    use screenroom_backend::media::ArtworkVariant;
    let adapters = adapters();
    let (inputs, omissions) = mapping::branding(&adapters).unwrap();
    assert!(omissions.is_empty());
    assert_eq!(inputs.len(), 1);
    let original = mapping::artwork_image(&adapters, "tmdb", ArtworkVariant::Original).unwrap();
    let native = mapping::artwork_image(&adapters, "tmdb", ArtworkVariant::Native).unwrap();
    assert_eq!(original.mime(), "image/svg+xml");
    assert_eq!(native.mime(), "image/png");
    assert_eq!(
        original.bytes(),
        adapters.read_artwork("tmdb", false).unwrap()
    );
    assert_eq!(native.bytes(), adapters.read_artwork("tmdb", true).unwrap());
}

#[test]
fn metadata_only_update_changes_document_and_content_hash() {
    let a = adapters();
    let mut entry = a.jellyfin_item("sintel").unwrap();
    let before = mapping::item(&entry, "s", true).unwrap();
    entry.item.overview.push_str(" A changed description.");
    let after = mapping::item(&entry, "s", true).unwrap();
    assert_ne!(before.content_hash, after.content_hash);
    assert_ne!(input_hash(&before.document), input_hash(&after.document));
}
