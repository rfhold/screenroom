use screenroom_backend::{
    Uuid,
    catalog::{CatalogRepository, ItemWrite},
    embedding::{EmbeddingClient, EmbeddingConfig},
    ingest::{
        CheckpointWrite, Completion, Followup, IngestRepository, JobKind, PageCommit, PolicyWrite,
    },
    search::SearchRepository,
    sqlx::{
        self,
        postgres::{PgConnectOptions, PgPoolOptions},
    },
};
use screenroom_catalog::Catalog;
use screenroom_proto::{
    catalog::v1::{search_service_server::SearchService, *},
    common::v1::SearchMethods,
};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

fn item(id: &str, hash: u8) -> ItemWrite {
    ItemWrite {
        public_id: id.into(),
        kind: "movie".into(),
        parent_public_id: None,
        external_id: id.into(),
        source_revision: Some(format!("synthetic-r{hash}")),
        metadata: json!({"title":if id=="a" {"Lexical"} else {"Semantic"}}),
        provenance: json!({}),
        content_hash: vec![hash; 32],
        library_external_ids: vec!["movies".into()],
        external_ids: vec![],
        credits: vec![],
        document: format!(
            "{} version{hash}",
            if id == "a" { "lexical" } else { "elsewhere" }
        ),
        document_title: if id == "a" { "Lexical" } else { "Semantic" }.into(),
        document_version: 1,
    }
}
async fn page(ingest: &IngestRepository, policy: Uuid, items: Vec<ItemWrite>, followups: bool) {
    ingest.start_run(policy).await.unwrap();
    let claim = ingest.claim("g-search", 60).await.unwrap().unwrap();
    let jobs = if followups {
        items
            .iter()
            .map(|i| Followup {
                kind: JobKind::Embedding,
                dedupe_key: format!("embedding-{}", i.external_id),
                input: json!({"external_id":i.external_id}),
            })
            .collect()
    } else {
        vec![]
    };
    ingest
        .commit_page(
            &claim,
            &PageCommit {
                libraries: vec![],
                items,
                feeds: vec![],
                artwork_inputs: vec![],
                artwork_omissions: vec![],
                followups: jobs,
                checkpoint: CheckpointWrite {
                    scope_key: "movies".into(),
                    cursor: json!({}),
                    watermark: None,
                },
                requests: 1,
                completion: Completion::Complete {
                    evidence: json!({"synthetic":true}),
                },
            },
        )
        .await
        .unwrap();
}
fn request(semantic_only: bool) -> SearchRequest {
    SearchRequest {
        query: "lexical".into(),
        source_id: "synthetic".into(),
        library_id: "movies".into(),
        methods: Some(SearchMethods {
            text: !semantic_only,
            semantic: true,
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_G_TEST_DATABASE_URL PostgreSQL/pgvector"]
async fn exact_hybrid_and_explicit_degradation() {
    let url = std::env::var("SCREENROOM_G_TEST_DATABASE_URL").expect("task-owned database URL");
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let name = format!("screenroom_g_search_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect_with(url.parse::<PgConnectOptions>().unwrap().database(&name))
        .await
        .unwrap();
    screenroom_backend::db::migrate(&pool).await.unwrap();
    let catalog_repo = CatalogRepository::new(pool.clone());
    let ingest = IngestRepository::new(pool.clone());
    let search = SearchRepository::new(pool.clone());
    let source = catalog_repo
        .configure_source(
            "synthetic",
            "jellyfin",
            &json!({"adapter":"mock"}),
            &json!({}),
        )
        .await
        .unwrap();
    catalog_repo
        .configure_library(source.id, "movies", "movies", "Movies")
        .await
        .unwrap();
    let policy = ingest
        .create_policy(
            source.id,
            &PolicyWrite {
                enabled: true,
                selection_json: json!({"library_ids":["movies"]}),
                max_items: 100,
                max_requests: 100,
                incremental_seconds: 60,
                reconcile_seconds: 3600,
                enrich_metadata: false,
                enrich_people: false,
                enrich_artwork: false,
            },
        )
        .await
        .unwrap();
    page(&ingest, policy.id, vec![item("a", 1), item("b", 2)], true).await;
    let model = search
        .register_model("synthetic-model", "synthetic-revision", 1024)
        .await
        .unwrap();
    while let Some(claim) = ingest.claim("g-search", 60).await.unwrap() {
        let id = claim.input["external_id"].as_str().unwrap();
        let document = search
            .document_for_reference("synthetic", id)
            .await
            .unwrap()
            .unwrap();
        let mut vector = vec![0.0; 1024];
        vector[if id == "a" { 1 } else { 0 }] = 1.0;
        search
            .commit_embedding(
                &claim,
                document.id,
                model.id,
                &model.revision,
                &document.content_hash,
                &vector,
            )
            .await
            .unwrap();
    }
    search.activate_model(model.id).await.unwrap();
    let unavailable = Arc::new(AtomicBool::new(false));
    let incompatible = Arc::new(AtomicBool::new(false));
    let delayed = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let call_count = calls.clone();
    let delay = delayed.clone();
    let fail = unavailable.clone();
    let bad = incompatible.clone();
    let router = axum::Router::new().route(
        "/embeddings",
        axum::routing::post(move |axum::Json(body): axum::Json<serde_json::Value>| {
            let fail = fail.clone();
            let bad = bad.clone();
            let delay = delay.clone();
            let call_count = call_count.clone();
            async move {
                call_count.fetch_add(1, Ordering::SeqCst);
                if delay.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                assert_eq!(body["input"], json!(["lexical"]));
                assert_eq!(body["model"], "synthetic-model");
                if fail.load(Ordering::SeqCst) {
                    return (
                        axum::http::StatusCode::SERVICE_UNAVAILABLE,
                        axum::Json(json!({})),
                    );
                }
                let mut vector = vec![0.0; 1024];
                vector[0] = 1.0;
                if bad.load(Ordering::SeqCst) {
                    vector.pop();
                }
                (
                    axum::http::StatusCode::OK,
                    axum::Json(
                        json!({"model":"synthetic-model","data":[{"index":0,"embedding":vector}]}),
                    ),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let config = EmbeddingConfig {
        base_url: format!("http://{address}"),
        model: "synthetic-model".into(),
        api_key: None,
        dimensions: 1024,
        revision: "synthetic-revision".into(),
        timeout: Duration::from_secs(2),
    };
    let runtime = PgPoolOptions::new()
        .max_connections(3)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE screenroom_catalog")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(url.parse::<PgConnectOptions>().unwrap().database(&name))
        .await
        .unwrap();
    let service = Catalog::new(runtime.clone(), "http://localhost:50052".into(), None, None)
        .unwrap()
        .with_embedding(EmbeddingClient::new(config.clone()).unwrap());
    let before = calls.load(Ordering::SeqCst);
    for (query, title_only, text, fuzzy, expected) in [
        ("version1", false, true, false, vec!["a"]),
        ("version1", true, true, false, vec![]),
        ("lexical", true, true, false, vec!["a"]),
        ("semantik", true, false, true, vec!["b"]),
    ] {
        let mut restricted = request(false);
        restricted.query = query.into();
        restricted.methods = Some(SearchMethods {
            title_only,
            text,
            fuzzy,
            semantic: title_only,
        });
        let response = service
            .search(tonic::Request::new(restricted))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            response
                .items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        let effective = response.effective_methods.unwrap();
        assert_eq!(effective.title_only, title_only);
        assert!(!effective.semantic);
        assert!(response.degradation_reason.is_empty());
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        before,
        "title restrictions must not call embeddings"
    );
    let public_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let public_address = public_listener.local_addr().unwrap();
    let (public_stop, public_stopped) = tokio::sync::oneshot::channel();
    let public = tokio::spawn(screenroom_catalog::serve(
        public_listener,
        async {
            let _ = public_stopped.await;
        },
        vec![],
        service.clone(),
    ));
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{public_address}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client =
        screenroom_proto::catalog::v1::search_service_client::SearchServiceClient::new(channel);
    delayed.store(true, Ordering::SeqCst);
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM search.result_snapshots")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut timed = tonic::Request::new(request(true));
    timed.set_timeout(Duration::from_millis(40));
    let error = client.search(timed).await.unwrap_err();
    assert!(matches!(
        error.code(),
        tonic::Code::Cancelled | tonic::Code::DeadlineExceeded
    ));
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut socket = tokio::net::TcpStream::connect(public_address)
            .await
            .unwrap();
        let frame = b"\0\0\0\0\x0d\x0a\x07lexical\x22\x02\x20\x01";
        socket.write_all(format!("POST /screenroom.catalog.v1.SearchService/Search HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/grpc-web+proto\r\nX-Grpc-Web: 1\r\nGrpc-Timeout: 40m\r\nContent-Length: {}\r\n\r\n", frame.len()).as_bytes()).await.unwrap();
        socket.write_all(frame).await.unwrap();
        let mut response = vec![];
        tokio::time::timeout(Duration::from_secs(1), socket.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        let response = String::from_utf8_lossy(&response);
        assert!(
            response.contains("grpc-status: 1") || response.contains("grpc-status: 4"),
            "{response}"
        );
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM search.result_snapshots")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        before, after,
        "expired requests must not publish late snapshots"
    );
    delayed.store(false, Ordering::SeqCst);
    public_stop.send(()).unwrap();
    public.await.unwrap().unwrap();
    let hybrid = service
        .search(tonic::Request::new(request(false)))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(hybrid.items[0].id, "a");
    assert!(hybrid.effective_methods.unwrap().semantic);
    assert!(hybrid.degradation_reason.is_empty());
    let semantic = service
        .search(tonic::Request::new(request(true)))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        semantic
            .items
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>(),
        vec!["b", "a"]
    );
    assert!(semantic.effective_methods.unwrap().semantic);
    let mut first = request(true);
    first.page = Some(Page {
        size: 1,
        token: String::new(),
    });
    let first = service
        .search(tonic::Request::new(first))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first.items[0].id, "b");
    assert!(!first.next_page_token.is_empty());
    let mut second = request(true);
    second.page = Some(Page {
        size: 1,
        token: first.next_page_token.clone(),
    });
    let mut changed = second.clone();
    changed.query = "another".into();
    assert_eq!(
        service
            .search(tonic::Request::new(changed))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::InvalidArgument
    );
    unavailable.store(true, Ordering::SeqCst);
    let tail = service
        .search(tonic::Request::new(second))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(tail.items[0].id, "a");
    assert!(tail.effective_methods.unwrap().semantic);
    let degraded = service
        .search(tonic::Request::new(request(false)))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(degraded.degradation_reason, "embedding_unavailable");
    assert!(!degraded.effective_methods.unwrap().semantic);
    assert_eq!(degraded.items[0].id, "a");
    assert_eq!(
        service
            .search(tonic::Request::new(request(true)))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unavailable
    );
    unavailable.store(false, Ordering::SeqCst);
    incompatible.store(true, Ordering::SeqCst);
    assert_eq!(
        service
            .search(tonic::Request::new(request(false)))
            .await
            .unwrap()
            .into_inner()
            .degradation_reason,
        "embedding_incompatible"
    );
    incompatible.store(false, Ordering::SeqCst);
    let mut mismatched = config;
    mismatched.revision = "not-the-index-revision".into();
    let mismatched = service
        .clone()
        .with_embedding(EmbeddingClient::new(mismatched).unwrap());
    assert_eq!(
        mismatched
            .search(tonic::Request::new(request(false)))
            .await
            .unwrap()
            .into_inner()
            .degradation_reason,
        "embedding_incompatible"
    );
    delayed.store(true, Ordering::SeqCst);
    let before = calls.load(Ordering::SeqCst);
    let racing_service = service.clone();
    let racing = tokio::spawn(async move {
        racing_service
            .search(tonic::Request::new(request(false)))
            .await
            .unwrap()
            .into_inner()
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while calls.load(Ordering::SeqCst) == before {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    page(&ingest, policy.id, vec![item("a", 9)], false).await;
    let raced = racing.await.unwrap();
    if raced.effective_methods.unwrap().semantic {
        assert!(raced.degradation_reason.is_empty());
        assert_eq!(raced.items.len(), 2);
    } else {
        assert_eq!(raced.degradation_reason, "embedding_coverage_incomplete");
    }
    delayed.store(false, Ordering::SeqCst);
    let degraded = service
        .search(tonic::Request::new(request(false)))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(degraded.degradation_reason, "embedding_coverage_incomplete");
    assert!(!degraded.effective_methods.unwrap().semantic);
    assert_eq!(degraded.items.len(), 1);
    assert_eq!(
        service
            .search(tonic::Request::new(request(true)))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unavailable
    );
    stop.send(()).unwrap();
    server.await.unwrap();
    runtime.close().await;
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
