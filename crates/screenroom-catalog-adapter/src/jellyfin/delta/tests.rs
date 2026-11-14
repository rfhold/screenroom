use super::*;
use crate::catalog_delta::{Query, Scope, Timestamp, Window};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn fixture(
    responses: Vec<(&str, String)>,
) -> (LiveJellyfin, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let responses: Vec<_> = responses
        .into_iter()
        .map(|(status, body)| {
            format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        })
        .collect();
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0; 1024];
                let count = socket.read(&mut chunk).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&chunk[..count]);
                assert!(request.len() < 16384);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            requests.push(String::from_utf8(request).unwrap());
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        requests
    });
    let file = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "screenroom-delta-{}-{}",
        std::process::id(),
        address.port()
    ));
    tokio::fs::write(&file, "delta-secret").await.unwrap();
    super::super::tests::private_permissions(&file);
    let provider = LiveJellyfin::from_config(JellyfinConfig {
        source_key: "home".into(),
        base_url: format!("http://{address}/jellyfin/"),
        secret_file: file.clone(),
        timeout_ms: 1000,
        max_response_bytes: 16384,
    })
    .await
    .unwrap();
    tokio::fs::remove_file(file).await.unwrap();
    (provider, task)
}
fn version(value: &str) -> (&'static str, String) {
    (
        "200 OK",
        serde_json::json!({ "Version": value }).to_string(),
    )
}
fn library() -> Library {
    Library {
        id: "ignored".into(),
        external_id: "shows".into(),
        name: "Shows".into(),
        kind: LibraryType::Television,
    }
}
fn checkpoint(size: u32, cap: u32, pages: u32) -> Checkpoint {
    Checkpoint::new(
        Query::new(
            Scope::new("home", "shows").unwrap(),
            Window::new(
                Timestamp::parse("2026-11-12T10:00:00Z").unwrap(),
                Timestamp::parse("2026-11-12T10:05:00Z").unwrap(),
            )
            .unwrap(),
            size,
            cap,
            pages,
        )
        .unwrap(),
    )
}
fn page(items: serde_json::Value, total: u32, offset: u32) -> (&'static str, String) {
    (
        "200 OK",
        serde_json::json!({"Items": items, "TotalRecordCount": total, "StartIndex": offset})
            .to_string(),
    )
}
fn episode(id: &str) -> serde_json::Value {
    serde_json::json!({"Id": id, "Name":"Episode", "Type":"Episode", "ParentId":"season", "ImageTags":{"Primary":"first"}})
}

#[tokio::test]
async fn durable_checkpoint_resumes_http_paging_without_replaying_offset_zero() {
    let (p, task) = fixture(vec![
        version("10.11.0"),
        page(serde_json::json!([episode("a")]), 2, 0),
        version("10.11.0"),
        page(serde_json::json!([episode("b")]), 2, 1),
    ])
    .await;
    let mut interrupted = checkpoint(1, 2, 2);
    p.library_delta_page(&library(), &mut interrupted)
        .await
        .unwrap();
    let bytes = interrupted.serialize().unwrap();
    let expected = interrupted.query().clone();
    let mut restored = Checkpoint::restore(&expected, &bytes).unwrap();
    let page = p
        .library_delta_page(&library(), &mut restored)
        .await
        .unwrap();
    assert_eq!(page.page.offset, 1);
    assert!(restored.is_complete());
    assert_eq!(restored.observed_count(), 2);
    assert!(!page.page.authoritative);
    let requests = task.await.unwrap();
    assert!(requests[1].contains("StartIndex=0"));
    assert!(requests[3].contains("StartIndex=1"));
}

#[tokio::test]
async fn supported_query_is_scoped_bounded_and_not_authoritative() {
    let (p, task) = fixture(vec![
        version("10.11.0"),
        page(serde_json::json!([episode("a")]), 2, 0),
        version("10.11.0"),
        page(serde_json::json!([episode("b")]), 2, 1),
    ])
    .await;
    let mut checkpoint = checkpoint(1, 2, 2);
    let first = p
        .library_delta_page(&library(), &mut checkpoint)
        .await
        .unwrap();
    assert!(!first.page.authoritative);
    assert_eq!(first.page.next_offset, Some(1));
    assert_eq!(first.parent_external_ids, ["season"]);
    assert_eq!(first.page.items[0].item.id, p.public_id("a"));
    let second = p
        .library_delta_page(&library(), &mut checkpoint)
        .await
        .unwrap();
    assert!(!second.page.authoritative);
    assert!(checkpoint.is_complete());
    assert_eq!(checkpoint.observed_count(), 2);
    assert_eq!(
        p.library_delta_page(&library(), &mut checkpoint)
            .await
            .unwrap_err(),
        DeltaError::Validation(ValidationError::Exhausted)
    );
    let requests = task.await.unwrap();
    for (index, request) in requests.iter().enumerate() {
        if index % 2 == 0 {
            assert!(request.starts_with("GET /jellyfin/System/Info/Public "));
            continue;
        }
        let url = Url::parse(&format!(
            "http://fixture{}",
            request.split_whitespace().nth(1).unwrap()
        ))
        .unwrap();
        let params: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(params["MinDateLastSaved"], "2026-11-12T10:00:00.0000000Z");
        assert_eq!(params["SortBy"], "SortName,DateCreated");
        assert_eq!(params["ParentId"], "shows");
        assert_eq!(params["Limit"], "1");
        assert_eq!(params["Recursive"], "true");
        assert_eq!(params["EnableUserData"], "false");
        assert!(!params.contains_key("UserId"));
        assert!(!params.contains_key("MinDateCreated"));
    }
}

#[tokio::test]
async fn unsafe_unknown_and_invalid_versions_fail_closed_before_items() {
    for (value, reason) in [
        ("10.10.7", FallbackReason::UnsupportedVersion),
        ("10.12.0", FallbackReason::UnsupportedVersion),
        ("11.0.0", FallbackReason::UnsupportedVersion),
        ("10.11.0-rc1", FallbackReason::InvalidVersion),
        ("10.11", FallbackReason::InvalidVersion),
        ("10.11.x", FallbackReason::InvalidVersion),
    ] {
        let (p, task) = fixture(vec![version(value)]).await;
        let mut checkpoint = checkpoint(1, 2, 2);
        assert_eq!(
            p.library_delta_page(&library(), &mut checkpoint)
                .await
                .unwrap_err(),
            DeltaError::FullScanRequired(reason)
        );
        assert_eq!(checkpoint.next_offset(), Some(0));
        assert_eq!(task.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn rejected_filters_and_transient_failures_do_not_advance() {
    for (status, expected) in [
        (
            "400 Bad Request",
            DeltaError::FullScanRequired(FallbackReason::FilterRejected),
        ),
        (
            "501 Not Implemented",
            DeltaError::FullScanRequired(FallbackReason::FilterRejected),
        ),
        (
            "503 Service Unavailable",
            DeltaError::Provider(Error::Http(503)),
        ),
        (
            "429 Too Many Requests",
            DeltaError::Provider(Error::RateLimited {
                retry_after_seconds: None,
            }),
        ),
        ("200 OK", DeltaError::Provider(Error::InvalidResponse)),
    ] {
        let (p, task) = fixture(vec![
            version("10.11.0"),
            (status, "private-error-body".into()),
        ])
        .await;
        let mut checkpoint = checkpoint(1, 2, 2);
        let error = p
            .library_delta_page(&library(), &mut checkpoint)
            .await
            .unwrap_err();
        assert_eq!(error, expected);
        assert!(!format!("{error:?}").contains("private-error-body"));
        assert_eq!(checkpoint.observed_count(), 0);
        assert_eq!(checkpoint.next_offset(), Some(0));
        task.await.unwrap();
    }
}

#[tokio::test]
async fn repeated_ids_changed_totals_and_bad_bounds_are_rejected_atomically() {
    for response in [
        page(serde_json::json!([episode("a")]), 2, 1),
        page(serde_json::json!([episode("b")]), 3, 1),
        page(serde_json::json!([episode("b")]), 2, 0),
        page(serde_json::json!([]), 2, 1),
        page(serde_json::json!([episode("b"), episode("c")]), 3, 1),
    ] {
        let (p, task) = fixture(vec![
            version("10.11.0"),
            page(serde_json::json!([episode("a")]), 2, 0),
            version("10.11.0"),
            response,
        ])
        .await;
        let mut checkpoint = checkpoint(1, 5, 5);
        p.library_delta_page(&library(), &mut checkpoint)
            .await
            .unwrap();
        assert!(
            p.library_delta_page(&library(), &mut checkpoint)
                .await
                .is_err()
        );
        assert_eq!(checkpoint.observed_count(), 1);
        assert_eq!(checkpoint.next_offset(), Some(1));
        task.await.unwrap();
    }
}

#[tokio::test]
async fn empty_delta_is_success_but_not_deletion_evidence() {
    let (p, task) = fixture(vec![version("10.11.0"), page(serde_json::json!([]), 0, 0)]).await;
    let mut checkpoint = checkpoint(1, 1, 1);
    let result = p
        .library_delta_page(&library(), &mut checkpoint)
        .await
        .unwrap();
    assert!(result.page.items.is_empty());
    assert!(!result.page.authoritative);
    assert!(checkpoint.is_complete());
    task.await.unwrap();
}

#[tokio::test]
async fn metadata_revision_ignores_personal_state_but_tracks_artwork_and_ancestry() {
    let mut rows = Vec::new();
    let original = episode("a");
    let mut watched = original.clone();
    watched["UserData"] = serde_json::json!({"Played":true,"PlaybackPositionTicks":0});
    let mut artwork = original.clone();
    artwork["ImageTags"]["Primary"] = "second".into();
    let mut metadata = original.clone();
    metadata["Name"] = "Renamed".into();
    let mut ancestry = original.clone();
    ancestry["ParentId"] = "other-season".into();
    for item in [original, watched, artwork, metadata, ancestry] {
        rows.push(version("10.11.0"));
        rows.push(page(serde_json::json!([item]), 1, 0));
    }
    let (p, task) = fixture(rows).await;
    let mut revisions = Vec::new();
    for _ in 0..5 {
        let result = p
            .library_delta_page(&library(), &mut checkpoint(1, 1, 1))
            .await
            .unwrap();
        revisions.push(result.page.items[0].revision.clone());
    }
    assert_eq!(revisions[0], revisions[1]);
    for revision in &revisions[2..] {
        assert_ne!(revisions[0], *revision);
    }
    assert!(!format!("{:?}", revisions[0]).contains("Episode"));
    task.await.unwrap();
}

#[tokio::test]
async fn hierarchy_and_child_refresh_are_explicit() {
    for item in [
        serde_json::json!({"Id":"season", "Type":"Season", "Name":"Season", "ParentId":"series"}),
        serde_json::json!({"Id":"series", "Type":"Series", "Name":"Series", "ParentId":"shows"}),
    ] {
        let (p, task) = fixture(vec![
            version("10.11.0"),
            page(serde_json::json!([item.clone()]), 1, 0),
        ])
        .await;
        let result = p
            .library_delta_page(&library(), &mut checkpoint(1, 1, 1))
            .await
            .unwrap();
        assert_eq!(
            result.refresh_children_external_ids,
            [item["Id"].as_str().unwrap()]
        );
        task.await.unwrap();
    }
    for parent in ["a", "shows"] {
        let mut item = episode("a");
        item["ParentId"] = parent.into();
        let (p, task) = fixture(vec![
            version("10.11.0"),
            page(serde_json::json!([item]), 1, 0),
        ])
        .await;
        assert_eq!(
            p.library_delta_page(&library(), &mut checkpoint(1, 1, 1))
                .await
                .unwrap_err(),
            DeltaError::Provider(Error::Hierarchy)
        );
        task.await.unwrap();
    }
}

#[test]
fn timestamp_window_query_and_version_validation() {
    for value in [
        "",
        "2026-11-12",
        "2026-11-12T00:00:00+00:00",
        "2026-02-29T00:00:00Z",
        "2024-02-30T00:00:00Z",
        "0000-01-01T00:00:00Z",
        "2026-11-12T24:00:00Z",
        "2026-11-12T00:60:00Z",
        "2026-11-12T00:00:60Z",
        "2026-11-12T00:00:00.Z",
        "2026-11-12T00:00:00.12345678Z",
        "é026-11-12T00:00:00Z",
    ] {
        assert!(Timestamp::parse(value).is_err(), "{value}");
    }
    let low = Timestamp::parse("2024-02-29T00:00:00.1Z").unwrap();
    assert_eq!(low.as_str(), "2024-02-29T00:00:00.1000000Z");
    let high = Timestamp::parse("2024-02-29T00:00:01Z").unwrap();
    assert!(Window::new(high.clone(), low.clone()).is_err());
    for (size, cap, pages) in [
        (0, 1, 1),
        (101, 200, 1),
        (2, 1, 1),
        (1, 50001, 1),
        (1, 1, 0),
    ] {
        assert!(
            Query::new(
                Scope::new("home", "shows").unwrap(),
                Window::new(low.clone(), high.clone()).unwrap(),
                size,
                cap,
                pages
            )
            .is_err()
        );
    }
    assert!(Scope::new("home", "..").is_err());
    for version in ["10.11.0.0", "010.11.0", "10.11.0-beta", "10.11. 0"] {
        assert!(ServerVersion::parse(version).is_none());
    }
}

#[tokio::test]
async fn scope_and_page_budget_fail_before_network() {
    let (p, task) = fixture(vec![
        version("10.11.0"),
        page(serde_json::json!([episode("a")]), 2, 0),
    ])
    .await;
    let mut limited = checkpoint(1, 2, 1);
    let mut wrong = library();
    wrong.external_id = "other".into();
    assert_eq!(
        p.library_delta_page(&wrong, &mut limited)
            .await
            .unwrap_err(),
        DeltaError::Validation(ValidationError::Scope)
    );
    p.library_delta_page(&library(), &mut limited)
        .await
        .unwrap();
    assert_eq!(
        p.library_delta_page(&library(), &mut limited)
            .await
            .unwrap_err(),
        DeltaError::Validation(ValidationError::Bounds)
    );
    assert!(!limited.is_complete());
    task.await.unwrap();
}
