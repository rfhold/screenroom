use screenroom_client_core::*;
use serde_json::json;

fn saved_settings(source: &str) -> serde_json::Value {
    json!({
        "scope": {"source_id": "synthetic-source", "library_id": "opaque-library"},
        "source": source,
        "types": {"movie": false, "series": true, "episode": false},
        "methods": {"title_only": true, "text": false, "fuzzy": true, "semantic": false},
        "subtitle_preference": "transcription"
    })
}

#[test]
fn legacy_source_and_retired_subtitle_preserve_ordinary_settings() {
    for source in ["tmdb", "all", "jellyfin"] {
        let input = saved_settings(source);
        let decoded: ClientSettings = serde_json::from_value(input.clone()).unwrap();
        let normalized = normalize_saved_settings(decoded).unwrap();
        let output = serde_json::to_value(&normalized).unwrap();
        let mut expected = input;
        expected
            .as_object_mut()
            .unwrap()
            .remove("subtitle_preference");
        expected["source"] = json!(if source == "tmdb" { "all" } else { source });
        assert_eq!(output, expected);
        assert_eq!(validate_settings(normalized.clone()).unwrap(), normalized);
        assert_eq!(
            normalize_saved_settings(normalized.clone()).unwrap(),
            normalized
        );
        let roundtrip: ClientSettings = serde_json::from_value(output).unwrap();
        assert_eq!(normalize_saved_settings(roundtrip).unwrap(), normalized);
    }
}

#[test]
fn saved_settings_migration_does_not_repair_unrelated_invalid_preferences() {
    for source in ["unknown", "", "TMDB"] {
        let decoded = serde_json::from_value(saved_settings(source)).unwrap();
        assert!(matches!(
            normalize_saved_settings(decoded),
            Err(ClientError::InvalidArgument { .. })
        ));
    }
    let mut invalid_types = saved_settings("tmdb");
    invalid_types["types"] = json!({"movie": false, "series": false, "episode": false});
    let mut invalid_methods = saved_settings("tmdb");
    invalid_methods["methods"] =
        json!({"title_only": true, "text": false, "fuzzy": false, "semantic": true});
    for input in [invalid_types, invalid_methods] {
        let decoded = serde_json::from_value(input).unwrap();
        assert!(matches!(
            normalize_saved_settings(decoded),
            Err(ClientError::InvalidArgument { .. })
        ));
    }
    let legacy = serde_json::from_value(saved_settings("tmdb")).unwrap();
    assert!(
        validate_settings(legacy).is_err(),
        "active settings reject the retired source"
    );
    let mut malformed = saved_settings("tmdb");
    malformed["source"] = json!(42);
    assert!(serde_json::from_value::<ClientSettings>(malformed).is_err());
}

#[test]
fn absent_optional_preferences_are_not_replaced_with_defaults() {
    let input = json!({"source": "tmdb", "scope": null, "types": null, "methods": null});
    let decoded = serde_json::from_value(input).unwrap();
    let normalized = normalize_saved_settings(decoded).unwrap();
    assert_eq!(
        serde_json::to_value(normalized).unwrap(),
        json!({
            "source": "all", "scope": null, "types": null, "methods": null
        })
    );
}
