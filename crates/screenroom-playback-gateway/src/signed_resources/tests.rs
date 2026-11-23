use super::*;
use axum::{Router, body::Body, extract::State, http::header, response::Response};
use screenroom_catalog_adapter::jellyfin::{ApiKey, ConnectionConfig, LiveJellyfin};
use serde_json::json;
use std::sync::Arc;

const CAP: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct Fixture {
    adapter: provider::PlaybackAdapter,
    auth: provider::SessionAuth,
    root: provider::ResourceRef,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Fixture {
    async fn new(manifest: String) -> Self {
        async fn route(
            State(manifest): State<Arc<String>>,
            request: axum::extract::Request,
        ) -> Response {
            let body = match request.uri().path() {
                "/System/Info" => json!({"Id":"server","ServerName":"Owned","Version":"10.10.7"}).to_string(),
                "/Users/Me" => json!({"Id":"personal","Policy":{"IsDisabled":false}}).to_string(),
                "/Users/personal/Items/item" => json!({
                    "Id":"item","Type":"Movie","MediaSources":[{"Id":"source","Protocol":"File"}]
                }).to_string(),
                "/Items/item/PlaybackInfo" => json!({
                    "PlaySessionId":"play",
                    "MediaSources":[{
                        "Id":"source","Protocol":"File","Container":"ts",
                        "SupportsDirectPlay":false,"SupportsDirectStream":false,"SupportsTranscoding":true,
                        "TranscodingUrl":"/Videos/item/master.m3u8?MediaSourceId=source&PlaySessionId=play&VideoCodec=h264&AudioCodec=aac&SubtitleStreamIndex=-1&api_key=private-provider-token",
                        "DefaultAudioStreamIndex":1,
                        "MediaStreams":[{"Type":"Video","Index":0,"Codec":"h264"},{"Type":"Audio","Index":1,"Codec":"aac"}]
                    }]
                }).to_string(),
                "/Videos/item/master.m3u8" => return Response::builder()
                    .header(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")
                    .header(header::CONTENT_LENGTH, manifest.len())
                    .body(Body::from(manifest.as_str().to_owned())).unwrap(),
                other => panic!("unexpected owned fixture path: {other}"),
            };
            Response::builder()
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap()
        }
        struct Guard;
        impl provider::DispatchGuard for Guard {
            fn check<'a>(
                &'a self,
                _: provider::DispatchContext<'a>,
            ) -> provider::DispatchCheck<'a> {
                Box::pin(async { Ok(()) })
            }
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new().fallback(route).with_state(Arc::new(manifest));
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let intent = provider::AuthorizationIntent::new(
            provider::GatewayDeviceId::from_recorded(format!(
                "screenroom-playback-{}",
                "a".repeat(64)
            ))
            .unwrap(),
            "server".into(),
            "personal".into(),
            "",
        )
        .unwrap();
        let adapter = LiveJellyfin::from_api_key(
            ConnectionConfig {
                source_key: "owned-signed-resource-test".into(),
                base_url: format!("http://{address}"),
                timeout_ms: 1000,
                max_response_bytes: 1024 * 1024,
            },
            ApiKey::new("private-admin-token".into()).unwrap(),
        )
        .unwrap()
        .playback_adapter_fenced(
            intent.clone(),
            provider::PlaybackTimeouts::default(),
            Arc::new(Guard),
        )
        .unwrap();
        let auth = adapter
            .restore_session(
                intent,
                provider::SessionToken::from_private_store("private-provider-token".into())
                    .unwrap(),
            )
            .await
            .unwrap();
        let negotiated = adapter
            .negotiate(
                &auth,
                provider::NegotiationRequest {
                    item_id: "item",
                    capabilities: provider::ClientCapabilities {
                        mp4: true,
                        h264: true,
                        aac: true,
                        mse_hls: true,
                        webvtt: true,
                        ..Default::default()
                    },
                    position: provider::Milliseconds::ZERO,
                    selection: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(negotiated.resource.kind(), provider::ResourceKind::Playlist);
        Self {
            adapter,
            auth,
            root: negotiated.resource,
            task,
        }
    }
}

fn context(anchor: Uuid) -> Context {
    Context {
        session: Uuid::new_v4(),
        revision: 1,
        anchor,
    }
}

fn film(count: usize) -> String {
    let mut body = "#EXTM3U\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-TARGETDURATION:4\n".to_owned();
    for index in 0..count {
        body.push_str(&format!(
            "#EXTINF:3.003,\nhls1/main/{index}.ts?api_key=private-provider-token\n"
        ));
    }
    body.push_str("#EXT-X-ENDLIST\n");
    body
}

fn references(rewritten: &str) -> Vec<&str> {
    rewritten
        .split(['"', '\n'])
        .filter(|part| part.starts_with("/playback/v1/media/"))
        .collect()
}

fn parse_path(path: &str) -> (Uuid, ResourceQuery) {
    let (path, query) = path.split_once('?').unwrap();
    (
        Uuid::parse_str(path.rsplit('/').next().unwrap()).unwrap(),
        ResourceQuery::parse(Some(query)).unwrap(),
    )
}

fn prepare(
    fixture: &Fixture,
    graph: &ResourceGraph,
    state: &PlaylistState,
    key: &SigningKey,
    context: Context,
    parent: &provider::ResourceRef,
    body: &[u8],
) -> provider::Result<PendingPlaylist> {
    let mut draft = PlaylistDraft::new(graph, key, context, CAP);
    let mut budget = state.budget.clone();
    let rewritten = fixture.adapter.rewrite_playlist(
        &fixture.auth,
        parent,
        body,
        &mut budget,
        |reference| draft.map(reference),
    )?;
    Ok(draft.finish(rewritten, budget))
}

#[test]
fn hmac_binds_every_context_field_and_process_key_and_domain() {
    let key = SigningKey::generate().unwrap();
    let replacement = SigningKey::generate().unwrap();
    let context = context(Uuid::new_v4());
    let child = Uuid::new_v4();
    let signature: [u8; 32] = key.mac(context, child).finalize().into_bytes().into();
    assert!(key.verify(context, child, &signature));
    assert!(!replacement.verify(context, child, &signature));
    for wrong in [
        Context {
            session: Uuid::new_v4(),
            ..context
        },
        Context {
            revision: 2,
            ..context
        },
        Context {
            anchor: Uuid::new_v4(),
            ..context
        },
    ] {
        assert!(!key.verify(wrong, child, &signature));
    }
    assert!(!key.verify(context, Uuid::new_v4(), &signature));
    let mut tampered = signature;
    tampered[31] ^= 1;
    assert!(!key.verify(context, child, &tampered));
    let mut wrong_domain = key.0.clone();
    wrong_domain.update(b"other-domain");
    wrong_domain.update(context.session.as_bytes());
    wrong_domain.update(&context.revision.to_be_bytes());
    wrong_domain.update(context.anchor.as_bytes());
    wrong_domain.update(child.as_bytes());
    assert!(!key.verify(context, child, &wrong_domain.finalize().into_bytes().into()));
}

#[test]
fn query_rejects_unknown_duplicate_incomplete_encoded_and_oversized_fields() {
    let anchor = Uuid::new_v4();
    let base = format!("cap={CAP}&anchor={anchor}&sig={CAP}");
    assert!(ResourceQuery::parse(Some(&base)).is_ok());
    assert!(ResourceQuery::parse(Some(&format!("sig={CAP}&cap={CAP}&anchor={anchor}"))).is_ok());
    assert!(ResourceQuery::parse(Some(&format!("cap={CAP}"))).is_ok());
    for bad in [
        "".into(),
        base.clone() + "&",
        base.clone() + "&url=http://provider/",
        base.clone() + &format!("&cap={CAP}"),
        base.clone() + &format!("&anchor={anchor}"),
        base.clone() + &format!("&sig={CAP}"),
        format!("cap={CAP}&anchor={anchor}"),
        format!("cap={CAP}&sig={CAP}"),
        format!("anchor={anchor}&sig={CAP}"),
        base.replace("sig=", "sig=%"),
        base.replace("anchor=", "anchor=%"),
        base.replace("cap=", "%63ap="),
        base.replace(CAP, &"A".repeat(64)),
        base.replace(&anchor.to_string(), &Uuid::nil().to_string()),
        base + "extra",
        "x".repeat(257),
    ] {
        assert!(ResourceQuery::parse(Some(&bad)).is_err());
    }
    assert!(ResourceQuery::parse(None).is_err());
}

#[tokio::test]
async fn complete_2859_segment_playlist_uses_production_draft_and_publication_without_database() {
    let body = film(2859);
    let fixture = Fixture::new(body).await;
    let root = Uuid::new_v4();
    let context = context(root);
    let key = SigningKey::generate().unwrap();
    let mut graph = ResourceGraph::new(HashMap::from([(root, fixture.root.clone())])).unwrap();
    let mut state = PlaylistState::default();
    let mut draft = PlaylistDraft::new(&graph, &key, context, CAP);
    let mut budget = state.budget.clone();
    let rewritten = fixture
        .adapter
        .playlist(&fixture.auth, &fixture.root, &mut budget, |reference| {
            draft.map(reference)
        })
        .await
        .unwrap();
    assert_eq!(references(&rewritten).len(), 2859);
    for private in [
        "private-provider-token",
        "private-admin-token",
        "api_key",
        "hls1/",
        "http://",
    ] {
        assert!(!rewritten.contains(private));
    }
    let paths: Vec<_> = references(&rewritten)
        .into_iter()
        .map(str::to_owned)
        .collect();
    let pending = draft.finish(rewritten, budget);
    assert_eq!(pending.children(), 2859);
    assert_eq!(graph.entries.len(), 1);
    state.publish(&mut graph, root, pending).unwrap();
    assert_eq!(graph.entries.len(), 2860);
    assert_eq!(graph.durable_ids().copied().collect::<Vec<_>>(), vec![root]);
    for path in &paths {
        let (id, query) = parse_path(path);
        let (anchor, reference) = graph.resolve(&key, context.session, 1, id, &query).unwrap();
        assert_eq!(anchor, root);
        assert_eq!(reference.kind(), provider::ResourceKind::Segment);
        assert!(graph.resolve(&key, context.session, 2, id, &query).is_err());
        assert!(
            graph
                .resolve(
                    &SigningKey::generate().unwrap(),
                    context.session,
                    1,
                    id,
                    &query
                )
                .is_err()
        );
        let unsigned = ResourceQuery::parse(Some(&format!("cap={CAP}"))).unwrap();
        assert!(
            graph
                .resolve(&key, context.session, 1, id, &unsigned)
                .is_err()
        );
    }
    let root_query = ResourceQuery::parse(Some(&format!("cap={CAP}"))).unwrap();
    assert_eq!(
        graph
            .resolve(&key, context.session, 1, root, &root_query)
            .unwrap()
            .0,
        root
    );
    let unknown = Uuid::new_v4();
    let (_, query) = parse_path(&key.path(context, unknown, CAP));
    assert!(
        graph
            .resolve(&key, context.session, 1, unknown, &query)
            .is_err()
    );
    let (_, signed_root) = parse_path(&key.path(context, root, CAP));
    assert!(
        graph
            .resolve(&key, context.session, 1, root, &signed_root)
            .is_err()
    );
}

#[tokio::test]
async fn nested_uri_attributes_initialization_segments_and_subtitles_keep_durable_root_anchor() {
    let fixture = Fixture::new(String::new()).await;
    let root = Uuid::new_v4();
    let context = context(root);
    let key = SigningKey::generate().unwrap();
    let mut graph = ResourceGraph::new(HashMap::from([(root, fixture.root.clone())])).unwrap();
    let mut state = PlaylistState::default();
    let master = b"#EXTM3U\n#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"English\",URI=\"source/Subtitles/3/subtitles.m3u8?api_key=private-provider-token\"\n#EXT-X-STREAM-INF:BANDWIDTH=100,CODECS=\"avc1.42E01E,mp4a.40.2\",SUBTITLES=\"subs\",VIDEO-RANGE=SDR\nmain.m3u8?api_key=private-provider-token\n";
    let pending = prepare(
        &fixture,
        &graph,
        &state,
        &key,
        context,
        &fixture.root,
        master,
    )
    .unwrap();
    let paths: Vec<_> = references(&pending.rewritten)
        .into_iter()
        .map(str::to_owned)
        .collect();
    state.publish(&mut graph, root, pending).unwrap();
    let mut kinds = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let (parent_id, query) = parse_path(path);
        let (anchor, parent) = graph
            .resolve(&key, context.session, 1, parent_id, &query)
            .unwrap();
        assert_eq!(anchor, root);
        assert_ne!(anchor, parent_id);
        let body = if index == 0 {
            b"#EXTM3U\n#EXTINF:3,\n0.vtt?api_key=private-provider-token\n#EXT-X-ENDLIST\n"
                .as_slice()
        } else {
            b"#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4?api_key=private-provider-token\"\n#EXTINF:3,\na.ts?api_key=private-provider-token\n#EXT-X-ENDLIST\n".as_slice()
        };
        let pending = prepare(&fixture, &graph, &state, &key, context, &parent, body).unwrap();
        let children: Vec<_> = references(&pending.rewritten)
            .into_iter()
            .map(str::to_owned)
            .collect();
        state.publish(&mut graph, parent_id, pending).unwrap();
        for child in children {
            let (id, query) = parse_path(&child);
            let (anchor, reference) = graph.resolve(&key, context.session, 1, id, &query).unwrap();
            assert_eq!(anchor, root);
            assert_ne!(anchor, parent_id);
            kinds.push(reference.kind());
        }
    }
    assert!(kinds.contains(&provider::ResourceKind::Subtitle));
    assert!(kinds.contains(&provider::ResourceKind::Initialization));
    assert!(kinds.contains(&provider::ResourceKind::Segment));
}

#[tokio::test]
async fn final_fence_denial_or_request_timeout_does_not_publish_and_cache_hits_revalidate() {
    use crate::transport::finalize_playlist;
    use std::{cell::Cell, time::Duration};

    let fixture = Fixture::new(String::new()).await;
    let cached = Uuid::new_v4();
    let missing = Uuid::new_v4();
    let key = SigningKey::generate().unwrap();
    let mut graph = ResourceGraph::new(HashMap::from([
        (cached, fixture.root.clone()),
        (missing, fixture.root.clone()),
    ]))
    .unwrap();
    let mut state = PlaylistState::default();
    let initial = prepare(
        &fixture,
        &graph,
        &state,
        &key,
        context(cached),
        &fixture.root,
        film(1).as_bytes(),
    )
    .unwrap();
    state.publish(&mut graph, cached, initial).unwrap();
    let before_entries = graph.entries.len();
    let before_children = graph.children.clone();
    let before_cache = state.rewritten.clone();
    let before_bytes = state.bytes;
    let before_budget = state.budget.resources();

    for timeout in [false, true] {
        let pending = prepare(
            &fixture,
            &graph,
            &state,
            &key,
            context(missing),
            &fixture.root,
            film(2).as_bytes(),
        )
        .unwrap();
        assert_eq!(pending.children(), 2);
        let published = Cell::new(false);
        let check = async {
            if timeout {
                std::future::pending::<()>().await;
            }
            Err(StatusCode::NOT_FOUND)
        };
        let result = tokio::time::timeout(
            Duration::from_millis(1),
            finalize_playlist(check, || {
                published.set(true);
                state.publish(&mut graph, missing, pending)
            }),
        )
        .await;
        if timeout {
            assert!(result.is_err());
        } else {
            assert_eq!(result.unwrap(), Err(StatusCode::NOT_FOUND));
        }
        assert!(!published.get());
        assert_eq!(graph.entries.len(), before_entries);
        assert_eq!(graph.children, before_children);
        assert_eq!(state.rewritten, before_cache);
        assert_eq!(state.bytes, before_bytes);
        assert_eq!(state.budget.resources(), before_budget);
        // The shared budget's identity set, not only its count, must remain unchanged.
        assert_eq!(
            prepare(
                &fixture,
                &graph,
                &state,
                &key,
                context(missing),
                &fixture.root,
                film(2).as_bytes(),
            )
            .unwrap()
            .budget
            .resources(),
            2
        );
    }

    let cache_checks = Cell::new(0);
    let bytes_released = Cell::new(false);
    let hit = finalize_playlist(
        async {
            cache_checks.set(cache_checks.get() + 1);
            Err(StatusCode::NOT_FOUND)
        },
        || {
            bytes_released.set(true);
            Ok(())
        },
    )
    .await;
    assert_eq!(hit, Err(StatusCode::NOT_FOUND));
    assert_eq!(cache_checks.get(), 1);
    assert!(!bytes_released.get());
    assert_eq!(state.rewritten, before_cache);

    let pending = prepare(
        &fixture,
        &graph,
        &state,
        &key,
        context(missing),
        &fixture.root,
        film(2).as_bytes(),
    )
    .unwrap();
    let checks = Cell::new(0);
    finalize_playlist(
        std::future::poll_fn(|_| {
            checks.set(checks.get() + 1);
            assert_eq!(checks.get(), 1, "no fence poll after publication");
            std::task::Poll::Ready(Ok(()))
        }),
        || {
            assert_eq!(checks.get(), 1);
            state.publish(&mut graph, missing, pending)
        },
    )
    .await
    .unwrap();
    assert_eq!(checks.get(), 1);
    assert_eq!(graph.entries.len(), before_entries + 2);
    assert_eq!(state.rewritten.len(), before_cache.len() + 1);
    assert_eq!(state.budget.resources(), 2);
}

#[tokio::test]
async fn graph_counts_roots_accepts_exactly_8192_and_failed_staging_publishes_nothing() {
    let fixture = Fixture::new(String::new()).await;
    let root = Uuid::new_v4();
    let context = context(root);
    let key = SigningKey::generate().unwrap();
    let mut graph = ResourceGraph::new(HashMap::from([
        (root, fixture.root.clone()),
        (Uuid::new_v4(), fixture.root.clone()),
    ]))
    .unwrap();
    let mut state = PlaylistState::default();
    let too_many = film(8191);
    assert!(
        prepare(
            &fixture,
            &graph,
            &state,
            &key,
            context,
            &fixture.root,
            too_many.as_bytes()
        )
        .is_err()
    );
    assert_eq!(graph.entries.len(), 2);
    assert_eq!(state.budget.resources(), 0);
    assert!(state.rewritten.is_empty());
    let exact = film(8190);
    let pending = prepare(
        &fixture,
        &graph,
        &state,
        &key,
        context,
        &fixture.root,
        exact.as_bytes(),
    )
    .unwrap();
    assert_eq!(pending.children(), 8190);
    // A failed cache publication must not consume graph capacity or the shared adapter budget.
    state.bytes = CACHE_LIMIT;
    assert!(state.publish(&mut graph, root, pending).is_err());
    assert_eq!(graph.entries.len(), 2);
    assert_eq!(state.budget.resources(), 0);
    assert!(state.rewritten.is_empty());
    state.bytes = 0;
    // Use a short representation to exercise the independent graph bound, not the cache bound.
    let mut pending = prepare(
        &fixture,
        &graph,
        &state,
        &key,
        context,
        &fixture.root,
        exact.as_bytes(),
    )
    .unwrap();
    pending.rewritten = "#EXTM3U\n".into();
    state.publish(&mut graph, root, pending).unwrap();
    assert_eq!(graph.entries.len(), 8192);
    let body = b"#EXTM3U\n#EXTINF:3,\nnew.ts\n#EXT-X-ENDLIST\n";
    assert!(prepare(&fixture, &graph, &state, &key, context, &fixture.root, body).is_err());
    assert_eq!(graph.entries.len(), 8192);
    assert_eq!(state.budget.resources(), 8190);
}

#[tokio::test]
async fn duplicate_children_reuse_identity_and_invalid_manifest_does_not_publish_partial_state() {
    let fixture = Fixture::new(String::new()).await;
    let root = Uuid::new_v4();
    let context = context(root);
    let key = SigningKey::generate().unwrap();
    let mut graph = ResourceGraph::new(HashMap::from([(root, fixture.root.clone())])).unwrap();
    let mut state = PlaylistState::default();
    let duplicate = b"#EXTM3U\n#EXTINF:3,\na.ts\n#EXTINF:3,\na.ts\n#EXT-X-ENDLIST\n";
    let master = b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\nfirst.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=200\nsecond.m3u8\n";
    let pending = prepare(
        &fixture,
        &graph,
        &state,
        &key,
        context,
        &fixture.root,
        master,
    )
    .unwrap();
    let playlists: Vec<_> = references(&pending.rewritten)
        .into_iter()
        .map(str::to_owned)
        .collect();
    state.publish(&mut graph, root, pending).unwrap();
    for (index, path) in playlists.iter().enumerate() {
        let (id, query) = parse_path(path);
        let (_, parent) = graph.resolve(&key, context.session, 1, id, &query).unwrap();
        let pending = prepare(&fixture, &graph, &state, &key, context, &parent, duplicate).unwrap();
        assert_eq!(pending.children(), usize::from(index == 0));
        let paths = references(&pending.rewritten);
        assert_eq!(paths[0], paths[1]);
        state.publish(&mut graph, id, pending).unwrap();
    }
    assert_eq!(graph.entries.len(), 4);
    let invalid =
        b"#EXTM3U\n#EXTINF:3,\nnew.ts\n#EXT-X-UNKNOWN:URI=\"https://external/\"\n#EXT-X-ENDLIST\n";
    assert!(
        prepare(
            &fixture,
            &graph,
            &state,
            &key,
            context,
            &fixture.root,
            invalid
        )
        .is_err()
    );
    assert_eq!(graph.entries.len(), 4);
    assert_eq!(state.budget.resources(), 3);
    assert_eq!(state.rewritten.len(), 3);
}

#[tokio::test]
async fn child_authority_uses_only_its_durable_root_and_does_not_bypass_stream_fences() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct RootGuard {
        actual: Uuid,
        expected: Uuid,
        revoked: AtomicBool,
    }
    impl crate::transport::StreamFence for RootGuard {
        fn check(
            &self,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), StatusCode>> + Send + '_>> {
            Box::pin(async {
                assert_eq!(self.actual, self.expected);
                if self.revoked.load(Ordering::Acquire) {
                    Err(StatusCode::NOT_FOUND)
                } else {
                    Ok(())
                }
            })
        }
    }
    let fixture = Fixture::new(String::new()).await;
    let root = Uuid::new_v4();
    let other_root = Uuid::new_v4();
    let context = context(root);
    let key = SigningKey::generate().unwrap();
    let mut graph = ResourceGraph::new(HashMap::from([
        (root, fixture.root.clone()),
        (other_root, fixture.root.clone()),
    ]))
    .unwrap();
    let mut state = PlaylistState::default();
    let pending = prepare(
        &fixture,
        &graph,
        &state,
        &key,
        context,
        &fixture.root,
        film(1).as_bytes(),
    )
    .unwrap();
    let path = references(&pending.rewritten)[0].to_owned();
    state.publish(&mut graph, root, pending).unwrap();
    let (child, query) = parse_path(&path);
    let (anchor, _) = graph
        .resolve(&key, context.session, 1, child, &query)
        .unwrap();
    for wrong_anchor in [other_root, child] {
        let (_, wrong) = parse_path(&key.path(
            Context {
                anchor: wrong_anchor,
                ..context
            },
            child,
            CAP,
        ));
        assert!(
            graph
                .resolve(&key, context.session, 1, child, &wrong)
                .is_err()
        );
    }
    let guard = Arc::new(RootGuard {
        actual: anchor,
        expected: root,
        revoked: AtomicBool::new(false),
    });
    assert_eq!(
        crate::transport::checked_local(guard.clone(), async { Ok::<_, ()>(42) })
            .await
            .unwrap(),
        42
    );
    // SQL decides these revocations in production. A valid child signature must not bypass denial.
    for _cause in [
        "stop",
        "profile_change",
        "source_change",
        "expiry",
        "revision_change",
    ] {
        guard.revoked.store(true, Ordering::Release);
        let polled = AtomicBool::new(false);
        assert_eq!(
            crate::transport::checked_local(guard.clone(), async {
                polled.store(true, Ordering::Release);
                Ok::<_, ()>(())
            })
            .await
            .unwrap_err(),
            StatusCode::NOT_FOUND
        );
        assert!(!polled.load(Ordering::Acquire));
    }
}
