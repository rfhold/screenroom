use super::*;

fn config() -> JellyfinConfig {
    JellyfinConfig {
        source_key: "home".into(),
        base_url: "https://private.example.invalid/jellyfin/".into(),
        secret_file: "/never-read/private-token".into(),
        timeout_ms: 10_000,
        max_response_bytes: 8 * 1024 * 1024,
    }
}

#[test]
fn stable_normalization_and_private_debug() {
    let original = config();
    let expected = ConfigurationRevision::from_config(&original).unwrap();
    for base_url in [
        "https://PRIVATE.EXAMPLE.INVALID:443/jellyfin/",
        "https://private.example.invalid/jellyfin",
        "https://private.example.invalid/a/../jellyfin/",
    ] {
        let mut equivalent = original.clone();
        equivalent.base_url = base_url.into();
        equivalent.secret_file = "/never-read//./private-token".into();
        assert_eq!(
            ConfigurationRevision::from_config(&equivalent).unwrap(),
            expected
        );
    }
    assert_eq!(expected.as_str().len(), 64);
    assert_eq!(
        ConfigurationRevision::parse(expected.as_str()).unwrap(),
        expected
    );
    let debug = format!("{original:?} {expected:?}");
    for private in [
        "private.example.invalid",
        "private-token",
        expected.as_str(),
    ] {
        assert!(!debug.contains(private));
    }
}

#[test]
fn all_semantic_request_configuration_and_locators_are_bound() {
    let original = config();
    let expected = ConfigurationRevision::from_config(&original).unwrap();
    let mut alternatives = vec![];
    for base_url in [
        "http://private.example.invalid/jellyfin/",
        "https://other.example.invalid/jellyfin/",
        "https://private.example.invalid:8443/jellyfin/",
        "https://private.example.invalid/other/",
        "https://private.example.invalid/jellyfin//",
    ] {
        let mut changed = original.clone();
        changed.base_url = base_url.into();
        alternatives.push(changed);
    }
    let mut changed = original.clone();
    changed.secret_file = "/never-read/different-token".into();
    alternatives.push(changed);
    let mut changed = original.clone();
    changed.source_key = "different".into();
    alternatives.push(changed);
    let mut changed = original.clone();
    changed.timeout_ms += 1;
    alternatives.push(changed);
    let mut changed = original;
    changed.max_response_bytes += 1;
    alternatives.push(changed);
    for changed in alternatives {
        assert_ne!(
            ConfigurationRevision::from_config(&changed).unwrap(),
            expected
        );
    }
}

#[test]
fn revision_parsing_and_locator_validation_fail_sanitized() {
    for value in [
        "",
        "abcd",
        &"a".repeat(63),
        &"a".repeat(65),
        &"A".repeat(64),
        &"g".repeat(64),
        &format!("{}\n", "a".repeat(63)),
    ] {
        assert_eq!(
            ConfigurationRevision::parse(value).unwrap_err(),
            Error::InvalidConfig
        );
    }
    for path in [
        "relative-token",
        "/",
        "/secret/../private-token",
        "/secret/\nprivate-token",
    ] {
        let mut invalid = config();
        invalid.secret_file = path.into();
        assert_eq!(
            ConfigurationRevision::from_config(&invalid).unwrap_err(),
            Error::InvalidConfig
        );
    }
    for base_url in [
        "https://user:secret@private.example.invalid/",
        "https://private.example.invalid/?token=secret",
        "not-a-url",
    ] {
        let mut invalid = config();
        invalid.base_url = base_url.into();
        assert_eq!(
            ConfigurationRevision::from_config(&invalid).unwrap_err(),
            Error::InvalidConfig
        );
    }
}

#[tokio::test]
async fn live_instance_matches_locator_digest_and_ignores_credential_rotation() {
    let mut actual = config();
    actual.secret_file = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("screenroom-revision-{}", std::process::id()));
    tokio::fs::write(&actual.secret_file, "private-first-token")
        .await
        .unwrap();
    super::super::tests::private_permissions(&actual.secret_file);
    let expected = ConfigurationRevision::from_config(&actual).unwrap();
    let first = LiveJellyfin::from_config(actual.clone()).await.unwrap();
    assert_eq!(first.configuration_revision().unwrap(), expected);
    tokio::fs::write(&actual.secret_file, "private-second-token")
        .await
        .unwrap();
    let rotated = LiveJellyfin::from_config(actual.clone()).await.unwrap();
    assert_eq!(rotated.configuration_revision().unwrap(), expected);
    tokio::fs::remove_file(&actual.secret_file).await.unwrap();
    // Neither API reopens the token, and the connected configuration stays bound.
    assert_eq!(
        ConfigurationRevision::from_config(&actual).unwrap(),
        expected
    );
    assert_eq!(first.configuration_revision().unwrap(), expected);
    let mut changed = actual;
    changed.base_url = "https://changed.example.invalid/".into();
    assert_ne!(
        ConfigurationRevision::from_config(&changed).unwrap(),
        expected
    );
    let debug = format!("{first:?} {rotated:?} {expected:?}");
    for private in [
        "private-first-token",
        "private-second-token",
        "private.example.invalid",
        "screenroom-revision-",
    ] {
        assert!(!debug.contains(private));
    }
}
