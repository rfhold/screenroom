use super::*;

pub(super) fn private_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}
use screenroom_observability::transport::{
    BodyDirection, RequestKind, RequestOutcome, TransportObservation, TransportObserver,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[derive(Default)]
struct Observations {
    bytes: std::sync::atomic::AtomicU64,
    outcomes: std::sync::Mutex<Vec<RequestOutcome>>,
}
impl TransportObserver for Observations {
    fn body_chunk(&self, kind: RequestKind, direction: BodyDirection, bytes: u64) {
        assert_eq!(kind, RequestKind::Provider);
        assert_eq!(direction, BodyDirection::DownloadConsumed);
        self.bytes
            .fetch_add(bytes, std::sync::atomic::Ordering::Relaxed);
    }
    fn finished(&self, kind: RequestKind, outcome: RequestOutcome, _: Duration) {
        assert_eq!(kind, RequestKind::Provider);
        self.outcomes.lock().unwrap().push(outcome);
    }
}

#[tokio::test]
async fn transport_observes_json_partial_failure_and_cancellation() {
    for (body, headers, delay, outcome, bytes) in [
        ("[]", "", 0, RequestOutcome::Succeeded, 2),
        (
            "3\r\nabc\r\n",
            "Transfer-Encoding: chunked\r\n",
            0,
            RequestOutcome::Failed,
            3,
        ),
        ("[]", "", 100, RequestOutcome::Cancelled, 0),
    ] {
        let (p, task) = provider("200 OK", body, headers, delay, 1000, 1000).await;
        let observations = std::sync::Arc::new(Observations::default());
        let p = p.with_transport_observer(TransportObservation::new(observations.clone()));
        if delay > 0 {
            assert!(
                tokio::time::timeout(Duration::from_millis(10), p.profiles())
                    .await
                    .is_err()
            );
        } else if outcome == RequestOutcome::Succeeded {
            p.profiles().await.unwrap();
        } else {
            assert!(p.profiles().await.is_err());
        }
        task.await.unwrap();
        assert_eq!(
            observations
                .bytes
                .load(std::sync::atomic::Ordering::Relaxed),
            bytes
        );
        assert_eq!(*observations.outcomes.lock().unwrap(), [outcome]);
    }
}

async fn provider(
    status: &str,
    body: &str,
    headers: &str,
    delay: u64,
    cap: usize,
    timeout: u64,
) -> (LiveJellyfin, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n{body}",
        body.len()
    );
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 8192];
        let n = socket.read(&mut request).await.unwrap();
        tokio::time::sleep(Duration::from_millis(delay)).await;
        let _ = socket.write_all(response.as_bytes()).await;
        String::from_utf8(request[..n].to_vec()).unwrap()
    });
    let file = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "screenroom-jellyfin-{}-{}",
        std::process::id(),
        address.port()
    ));
    tokio::fs::write(&file, "test-private-key\n").await.unwrap();
    private_permissions(&file);
    let result = LiveJellyfin::from_config(JellyfinConfig {
        source_key: "home".into(),
        base_url: format!("http://{address}/jellyfin/"),
        secret_file: file.clone(),
        timeout_ms: timeout,
        max_response_bytes: cap,
    })
    .await
    .unwrap();
    tokio::fs::remove_file(file).await.unwrap();
    (result, task)
}

#[tokio::test]
async fn auth_basepath_hidden_profiles_and_redaction() {
    let (p, task) = provider("200 OK", r#"[{"Id":"external-uuid","Name":"Hidden","Policy":{"IsDisabled":false,"IsHidden":true}},{"Id":"disabled","Name":"No","Policy":{"IsDisabled":true,"IsHidden":false}}]"#, "", 0, 4096, 1000).await;
    let profiles = p.profiles().await.unwrap();
    assert_eq!(profiles.len(), 1);
    assert!(profiles[0].hidden);
    let request = task.await.unwrap();
    assert!(request.starts_with("GET /jellyfin/Users "));
    assert!(request.contains("MediaBrowser Token=\"test-private-key\""));
    assert!(!format!("{p:?}").contains("test-private-key"));
}

#[tokio::test]
async fn transport_is_bounded_and_never_follows_redirects() {
    for (status, body, headers, delay, cap, timeout, expected) in [
        (
            "302 Found",
            "",
            "Location: http://127.0.0.1:1/leak\r\n",
            0,
            100,
            1000,
            Error::Http(302),
        ),
        (
            "429 Too Many Requests",
            "",
            "Retry-After: 17\r\n",
            0,
            100,
            1000,
            Error::RateLimited {
                retry_after_seconds: Some(17),
            },
        ),
        (
            "200 OK",
            "a very long body",
            "",
            0,
            2,
            1000,
            Error::TooLarge,
        ),
        ("200 OK", "[]", "", 100, 100, 10, Error::Deadline),
        (
            "401 Unauthorized",
            "secret-error-body",
            "",
            0,
            100,
            1000,
            Error::Unauthorized,
        ),
        ("200 OK", "{}", "", 0, 100, 1000, Error::InvalidResponse),
    ] {
        let (p, task) = provider(status, body, headers, delay, cap, timeout).await;
        assert_eq!(p.profiles().await.unwrap_err(), expected);
        task.await.unwrap();
    }
}

#[tokio::test]
async fn catalog_page_has_arbitrary_ids_optional_providers_and_non_authoritative_evidence() {
    let (p, task) = provider("200 OK", r#"{"Items":[{"Id":"episode-uuid","Name":"Episode","Type":"Episode","ParentId":"season-uuid","ProviderIds":{}}],"TotalRecordCount":77,"StartIndex":50}"#, "", 0, 4096, 1000).await;
    let library = Library {
        id: "ignored".into(),
        external_id: "shows-uuid".into(),
        name: "Shows".into(),
        kind: LibraryType::Television,
    };
    let page = p.library_page(&library, 50, 1).await.unwrap();
    assert_eq!(page.next_offset, Some(51));
    assert!(!page.authoritative);
    assert_eq!(
        page.items[0].parent_external_id.as_deref(),
        Some("season-uuid")
    );
    assert!(page.items[0].provider_ids.is_empty());
    assert!(task.await.unwrap().contains("StartIndex=50"));
}

#[tokio::test]
async fn resume_pages_past_fifty_and_tick_guards() {
    let (p, task) = provider("200 OK", r#"{"Items":[{"Id":"movie-uuid","Name":"Movie","Type":"Movie","RunTimeTicks":100000000,"UserData":{"PlaybackPositionTicks":50000000,"Played":false}}],"TotalRecordCount":60,"StartIndex":51}"#, "", 0, 4096, 1000).await;
    let page = p.resume_page("profile", "movies", 51, 1).await.unwrap();
    assert_eq!(page.items[0].position_seconds, 5);
    assert_eq!(page.next_offset, Some(52));
    let request = task.await.unwrap();
    assert!(request.contains("ParentId=movies"));
    assert!(request.contains("/Users/profile/Items/Resume?"));
    assert_eq!(ticks(-1), Err(Error::InvalidResponse));
    assert_eq!(ticks(i64::MAX), Ok(922337203685));
}

#[tokio::test]
async fn metadata_roots_do_not_lookup_library_parents() {
    let (p, task) = provider("200 OK", r#"{"Items":[{"Id":"movie","Name":"Movie","Type":"Movie","ParentId":"library","ProviderIds":{"Imdb":"tt123"}}]}"#, "", 0, 4096, 1000).await;
    let item = p.item_by_external_id("movie").await.unwrap();
    assert!(item.parent_external_id.is_none());
    assert_eq!(item.provider_ids["Imdb"], "tt123");
    assert!(p.ancestors(&item, 8).await.unwrap().is_empty());
    task.await.unwrap();
}

#[tokio::test]
async fn invalid_config_fails_before_loading_secrets() {
    for base in [
        "http://user:password@localhost",
        "http://localhost/?token=secret",
        "http://localhost/#fragment",
        "file:///tmp/test",
        "http://localhost/\n",
    ] {
        let result = LiveJellyfin::from_config(JellyfinConfig {
            source_key: "home".into(),
            base_url: base.into(),
            secret_file: "/must-not-read".into(),
            timeout_ms: 100,
            max_response_bytes: 100,
        })
        .await;
        assert_eq!(result.unwrap_err(), Error::InvalidConfig);
    }
}

#[tokio::test]
async fn artwork_preserves_exact_bytes_and_uses_item_image_endpoint() {
    let (p, task) = provider("200 OK", "image-original", "", 0, 100, 1000).await;
    let bytes = p
        .artwork_bytes(&Artwork {
            item_id: "external-item".into(),
            kind: "Primary".into(),
            tag: "revision".into(),
        })
        .await
        .unwrap();
    assert_eq!(bytes, b"image-original");
    let request = task.await.unwrap();
    assert!(request.starts_with("GET /jellyfin/Items/external-item/Images/Primary?tag=revision "));
}

#[tokio::test]
async fn ancestor_cycle_is_rejected() {
    let (p, task) = provider(
        "200 OK",
        r#"{"Items":[{"Id":"parent","Name":"Season","Type":"Season","ParentId":"child"}]}"#,
        "",
        0,
        4096,
        1000,
    )
    .await;
    let child = p
        .item(
            serde_json::from_str(
                r#"{"Id":"child","Name":"Episode","Type":"Episode","ParentId":"parent"}"#,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(p.ancestors(&child, 8).await.unwrap_err(), Error::Hierarchy);
    task.await.unwrap();
}

#[tokio::test]
async fn ancestor_response_identity_must_match_requested_parent() {
    let (p, task) = provider(
        "200 OK",
        r#"{"Items":[{"Id":"unrelated","Name":"Series","Type":"Series"}]}"#,
        "",
        0,
        4096,
        1000,
    )
    .await;
    let child = p
        .item(
            serde_json::from_str(
                r#"{"Id":"child","Name":"Season","Type":"Season","ParentId":"requested-parent"}"#,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(p.ancestors(&child, 8).await.unwrap_err(), Error::Hierarchy);
    task.await.unwrap();
}

#[tokio::test]
async fn id_filtered_lookup_reconstructs_hierarchy_without_unsupported_direct_route() {
    let (mut p, unused) = provider("400 Bad Request", "", "", 0, 4096, 1000).await;
    unused.abort();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    p.base = Url::parse(&format!(
        "http://{}/jellyfin/",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let server = tokio::spawn(async move {
        for (id, body) in [
            (
                "episode",
                r#"{"Items":[{"Id":"episode","Name":"Episode","Type":"Episode","ParentId":"season"}]}"#,
            ),
            (
                "season",
                r#"{"Items":[{"Id":"season","Name":"Season","Type":"Season","ParentId":"series"}]}"#,
            ),
            (
                "series",
                r#"{"Items":[{"Id":"series","Name":"Series","Type":"Series","ParentId":"library"}]}"#,
            ),
        ] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 8192];
            let n = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..n]);
            let filtered = request.starts_with(&format!("GET /jellyfin/Items?Ids={id}&"));
            let (status, response) = if filtered {
                ("200 OK", body)
            } else {
                ("400 Bad Request", "")
            };
            socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
            assert!(
                filtered,
                "must use one filtered request, not the unsupported direct route"
            );
            assert!(request.contains("Fields=Overview%2CProviderIds%2CParentId&Limit=1"));
        }
    });
    let episode = p.item_by_external_id("episode").await.unwrap();
    let ancestors = p.ancestors(&episode, 8).await.unwrap();
    assert_eq!(
        ancestors
            .iter()
            .map(|i| i.external_id.as_str())
            .collect::<Vec<_>>(),
        ["series", "season"]
    );
    assert_eq!(ancestors[1].parent_external_id.as_deref(), Some("series"));
    assert!(ancestors[0].parent_external_id.is_none());
    server.await.unwrap();
}

#[tokio::test]
async fn id_filtered_lookup_rejects_missing_malformed_and_duplicate_rows() {
    for (body, expected) in [
        (r#"{"Items":[]}"#, Error::NotFound),
        (r#"{}"#, Error::InvalidResponse),
        (r#"{"Items":[{"Id":"item"}]}"#, Error::InvalidResponse),
        (
            r#"{"Items":[{"Id":"item","Name":"A","Type":"Movie"},{"Id":"item","Name":"B","Type":"Movie"}]}"#,
            Error::InvalidResponse,
        ),
        (
            r#"{"Items":[{"Id":"other","Name":"A","Type":"Movie"}]}"#,
            Error::Hierarchy,
        ),
    ] {
        let (p, task) = provider("200 OK", body, "", 0, 4096, 1000).await;
        assert_eq!(p.item_by_external_id("item").await.unwrap_err(), expected);
        assert!(
            task.await
                .unwrap()
                .starts_with("GET /jellyfin/Items?Ids=item&")
        );
    }
}

#[tokio::test]
async fn movies_offset_seventeen_explicitly_disables_collection_collapsing() {
    let (mut p, unused) = provider("400 Bad Request", "", "", 0, 4096, 1000).await;
    unused.abort();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    p.base = Url::parse(&format!(
        "http://{}/jellyfin/",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 8192];
        let n = socket.read(&mut request).await.unwrap();
        let request = String::from_utf8_lossy(&request[..n]);
        let target = request.split_whitespace().nth(1).unwrap();
        let url = Url::parse(&format!("http://localhost{target}")).unwrap();
        let query: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
        let kind = if query.get("CollapseBoxSetItems").map(String::as_str) == Some("false") {
            "Movie"
        } else {
            "BoxSet"
        };
        let body = format!(
            r#"{{"Items":[{{"Id":"movie","Name":"Movie","Type":"{kind}","ParentId":"movies"}}],"TotalRecordCount":2889,"StartIndex":17}}"#
        );
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        assert_eq!(url.path(), "/jellyfin/Items");
        let expected = BTreeMap::from([
            ("ParentId", "movies"),
            ("Recursive", "true"),
            ("CollapseBoxSetItems", "false"),
            ("StartIndex", "17"),
            ("Limit", "1"),
            ("IncludeItemTypes", "Movie,Series,Season,Episode"),
            ("Fields", "Overview,ProviderIds,ParentId"),
            ("EnableTotalRecordCount", "true"),
            ("SortBy", "SortName,Id"),
            ("SortOrder", "Ascending"),
        ]);
        assert_eq!(
            query
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect::<BTreeMap<_, _>>(),
            expected
        );
    });
    let library = Library {
        id: "public-movies".into(),
        external_id: "movies".into(),
        name: "Movies".into(),
        kind: LibraryType::Movies,
    };
    let page = p.library_page(&library, 17, 1).await.unwrap();
    assert_eq!(page.items[0].kind, MediaKind::Movie);
    assert_eq!(page.reported_total, 2889);
    assert_eq!(page.next_offset, Some(18));
    assert!(!page.authoritative);
    server.await.unwrap();
}

#[tokio::test]
async fn collections_and_user_context_members_paginate_without_reparenting() {
    let (mut p, unused) = provider("400 Bad Request", "", "", 0, 64 * 1024, 1000).await;
    unused.abort();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    p.base = Url::parse(&format!(
        "http://{}/jellyfin/",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let server = tokio::spawn(async move {
        for _ in 0..6 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 8192];
            let n = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..n]);
            let url = Url::parse(&format!(
                "http://localhost{}",
                request.split_whitespace().nth(1).unwrap()
            ))
            .unwrap();
            let q: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
            assert_eq!(q["CollapseBoxSetItems"], "false");
            assert_eq!(q["SortBy"], "SortName,Id");
            let offset: usize = q["StartIndex"].parse().unwrap();
            let limit: usize = q["Limit"].parse().unwrap();
            let (total, items) = if q["ParentId"] == "collections" {
                assert_eq!(q["Recursive"], "true");
                assert_eq!(q["IncludeItemTypes"], "BoxSet");
                (208, (offset..(offset+limit).min(208)).map(|i| serde_json::json!({"Id":format!("box{i}"),"Name":"Collection","Type":"BoxSet","ParentId":"library","ProviderIds":null,"ImageTags":null})).collect::<Vec<_>>())
            } else {
                assert_eq!(q["Recursive"], "false");
                assert_eq!(q["EnableUserData"], "false");
                assert!(!q.contains_key("IncludeItemTypes"));
                let total = if q.get("UserId").map(String::as_str) == Some("operator-profile") {
                    3
                } else {
                    0
                };
                (total, (offset..(offset+limit).min(total)).map(|i| serde_json::json!({"Id":format!("member{i}"),"Name":"Member","Type":if i==2 {"Audio"} else {"Movie"},"ParentId":"canonical-library"})).collect())
            };
            let body =
                serde_json::json!({"Items":items,"TotalRecordCount":total,"StartIndex":offset})
                    .to_string();
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
    });
    let library = Library {
        id: "public-library".into(),
        external_id: "collections".into(),
        name: "Collections".into(),
        kind: LibraryType::Collections,
    };
    let mut offset = 0;
    let mut count = 0;
    loop {
        let page = p.collection_page(&library, offset, 100).await.unwrap();
        assert!(!page.authoritative);
        count += page.items.len();
        assert!(
            page.items
                .iter()
                .all(|i| i.provider_ids.is_empty() && i.id == p.public_id(&i.external_id))
        );
        match page.next_offset {
            Some(next) => {
                assert!(next > offset);
                offset = next
            }
            None => break,
        }
    }
    assert_eq!(count, 208);
    for offset in 0..3 {
        let page = p
            .collection_members_page("operator-profile", "box0", offset, 1)
            .await
            .unwrap();
        assert_eq!(page.reported_total, 3);
        assert!(!page.authoritative);
        assert_eq!(
            page.items[0].parent_external_id.as_deref(),
            Some("canonical-library")
        );
        assert_eq!(page.items[0].id, p.public_id(&page.items[0].external_id));
        if offset == 2 {
            assert_eq!(page.items[0].kind, MemberKind::Unsupported("Audio".into()));
            assert!(page.next_offset.is_none())
        } else {
            assert_eq!(page.next_offset, Some(offset + 1));
        }
    }
    server.await.unwrap();
}

#[tokio::test]
async fn collection_guards_metadata_and_checked_offsets() {
    let library = Library {
        id: "public".into(),
        external_id: "collections".into(),
        name: "Collections".into(),
        kind: LibraryType::Collections,
    };
    for body in [
        r#"{"Items":[],"TotalRecordCount":208,"StartIndex":0}"#,
        r#"{"Items":[],"TotalRecordCount":0,"StartIndex":1}"#,
        r#"{"Items":[{"Id":"a","Name":"A","Type":"BoxSet"},{"Id":"a","Name":"A","Type":"BoxSet"}],"TotalRecordCount":2}"#,
        r#"{"Items":[{"Id":"a","Name":" ","Type":"BoxSet"}],"TotalRecordCount":1}"#,
        r#"{"Items":[{"Id":"a","Name":"A","Type":"Series"}],"TotalRecordCount":1}"#,
    ] {
        let (p, task) = provider("200 OK", body, "", 0, 4096, 1000).await;
        assert_eq!(
            p.collection_page(&library, 0, 100).await.unwrap_err(),
            Error::InvalidResponse
        );
        task.await.unwrap();
    }
    let (p, unused) = provider("200 OK", "{}", "", 0, 4096, 1000).await;
    assert_eq!(
        p.collection_members_page("", "box", 0, 1)
            .await
            .unwrap_err(),
        Error::InvalidScope
    );
    assert_eq!(
        p.collection_members_page("profile", "", 0, 1)
            .await
            .unwrap_err(),
        Error::InvalidScope
    );
    assert!(page_query("library", 100_001, 100).is_ok());
    assert!(page_query("library", u32::MAX - 1, 1).is_ok());
    assert_eq!(
        page_query("library", u32::MAX, 1).unwrap_err(),
        Error::InvalidScope
    );
    let dto:ItemDto=serde_json::from_str(r#"{"Id":"movie","Name":"Movie","Type":"Movie","ProviderIds":null,"ImageTags":null,"BackdropImageTags":["first","second"]}"#).unwrap();
    let item = p.item(dto).unwrap();
    assert!(item.provider_ids.is_empty());
    assert_eq!(item.artwork.len(), 1);
    assert_eq!(item.artwork[0].kind, "Backdrop");
    assert_eq!(item.artwork[0].tag, "first");
    unused.abort();
}

#[tokio::test]
async fn collection_members_reject_self_identity() {
    let (p, task) = provider(
        "200 OK",
        r#"{"Items":[{"Id":"box","Name":"Self","Type":"BoxSet"}],"TotalRecordCount":1}"#,
        "",
        0,
        4096,
        1000,
    )
    .await;
    assert_eq!(
        p.collection_members_page("profile", "box", 0, 1)
            .await
            .unwrap_err(),
        Error::Hierarchy
    );
    task.await.unwrap();
}

#[tokio::test]
async fn collection_discovery_does_not_expand_media_kinds() {
    let (p, task) = provider(
        "200 OK",
        r#"[{"ItemId":"collections","Name":"Collections","CollectionType":"boxsets"}]"#,
        "",
        0,
        4096,
        1000,
    )
    .await;
    assert_eq!(
        p.libraries().await.unwrap()[0].kind,
        LibraryType::Collections
    );
    let dto: ItemDto =
        serde_json::from_str(r#"{"Id":"box","Name":"Collection","Type":"BoxSet"}"#).unwrap();
    assert_eq!(p.item(dto).unwrap_err(), Error::InvalidResponse);
    let dto: ItemDto =
        serde_json::from_str(r#"{"Id":"movie","Name":"\u0001 ","Type":"Movie"}"#).unwrap();
    assert_eq!(p.item(dto).unwrap_err(), Error::InvalidResponse);
    task.await.unwrap();
}
