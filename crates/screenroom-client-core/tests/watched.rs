use screenroom_client_core::Item;

#[test]
fn watched_optional_bool_is_not_progress_or_false_by_default() {
    for watched in [None, Some(false), Some(true)] {
        let wire = screenroom_proto::catalog::v1::Item {
            watched,
            progress_percent: Some(100),
            ..Default::default()
        };
        let value = serde_json::to_value(wire).unwrap();
        let item: Item = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(item.watched, watched);
        assert_eq!(item.progress_percent, Some(100));
        assert_eq!(
            serde_json::to_value(item).unwrap()["watched"],
            value["watched"]
        );
    }
    assert_eq!(Item::default().watched, None);
}

#[cfg(not(target_arch = "wasm32"))]
mod fresh_reads {
    use screenroom_client_core::{Client, ClientConfig, Page, SearchRequest, SourceLibraryScope};
    use screenroom_proto::catalog::v1 as wire;
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tonic::{Request, Response, Status};

    #[derive(Clone, Default)]
    struct Server {
        watched: Arc<Mutex<Option<bool>>>,
        counts: Arc<Mutex<HashMap<&'static str, usize>>>,
    }
    impl Server {
        async fn record(&self, op: &'static str) {
            *self.counts.lock().unwrap().entry(op).or_default() += 1;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        async fn item(&self, op: &'static str) -> wire::Item {
            self.record(op).await;
            let watched = *self.watched.lock().unwrap();
            wire::Item {
                id: "item".into(),
                watched,
                progress_percent: watched.map(|_| 42),
                ..Default::default()
            }
        }
        fn count(&self, op: &'static str) -> usize {
            self.counts.lock().unwrap()[op]
        }
    }
    #[tonic::async_trait]
    impl wire::browse_service_server::BrowseService for Server {
        async fn get_home(
            &self,
            _: Request<wire::GetHomeRequest>,
        ) -> Result<Response<wire::GetHomeResponse>, Status> {
            Ok(Response::new(wire::GetHomeResponse {
                rows: vec![wire::HomeRow {
                    items: vec![self.item("home").await],
                    ..Default::default()
                }],
            }))
        }
        async fn list_libraries(
            &self,
            _: Request<wire::ListLibrariesRequest>,
        ) -> Result<Response<wire::ListLibrariesResponse>, Status> {
            self.record("libraries").await;
            Ok(Response::new(Default::default()))
        }
        async fn list_library_items(
            &self,
            _: Request<wire::ListLibraryItemsRequest>,
        ) -> Result<Response<wire::ListLibraryItemsResponse>, Status> {
            Ok(Response::new(wire::ListLibraryItemsResponse {
                items: vec![self.item("items").await],
                ..Default::default()
            }))
        }
    }
    #[tonic::async_trait]
    impl wire::catalog_service_server::CatalogService for Server {
        async fn get_catalog_info(
            &self,
            _: Request<wire::GetCatalogInfoRequest>,
        ) -> Result<Response<wire::GetCatalogInfoResponse>, Status> {
            self.record("info").await;
            Ok(Response::new(wire::GetCatalogInfoResponse {
                info: Some(Default::default()),
            }))
        }
        async fn get_item(
            &self,
            _: Request<wire::GetItemRequest>,
        ) -> Result<Response<wire::GetItemResponse>, Status> {
            Ok(Response::new(wire::GetItemResponse {
                item: Some(self.item("item").await),
            }))
        }
        async fn list_children(
            &self,
            _: Request<wire::ListChildrenRequest>,
        ) -> Result<Response<wire::ListChildrenResponse>, Status> {
            Ok(Response::new(wire::ListChildrenResponse {
                items: vec![self.item("children").await],
                ..Default::default()
            }))
        }
        async fn list_credits(
            &self,
            _: Request<wire::ListCreditsRequest>,
        ) -> Result<Response<wire::ListCreditsResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }
        async fn list_artwork(
            &self,
            _: Request<wire::ListArtworkRequest>,
        ) -> Result<Response<wire::ListArtworkResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }
    }
    #[tonic::async_trait]
    impl wire::search_service_server::SearchService for Server {
        async fn get_search_capabilities(
            &self,
            _: Request<wire::GetSearchCapabilitiesRequest>,
        ) -> Result<Response<wire::GetSearchCapabilitiesResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }
        async fn search(
            &self,
            _: Request<wire::SearchRequest>,
        ) -> Result<Response<wire::SearchResponse>, Status> {
            Ok(Response::new(wire::SearchResponse {
                items: vec![self.item("search").await],
                ..Default::default()
            }))
        }
    }

    #[tokio::test]
    async fn repeated_item_reads_revalidate_expiry_and_external_unlink_but_keep_inflight_dedup() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let state = Server::default();
        let server = state.clone();
        let (stop, rx) = tokio::sync::oneshot::channel();
        let incoming = futures::stream::unfold(listener, |listener| async {
            Some((listener.accept().await.map(|(stream, _)| stream), listener))
        });
        let task = tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(wire::browse_service_server::BrowseServiceServer::new(
                    server.clone(),
                ))
                .add_service(wire::catalog_service_server::CatalogServiceServer::new(
                    server.clone(),
                ))
                .add_service(wire::search_service_server::SearchServiceServer::new(
                    server,
                ))
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = rx.await;
                }),
        );
        let client = Client::new(ClientConfig {
            endpoint,
            user_id: "00000000-0000-0000-0000-000000000001".into(),
            timeout_ms: 3000,
        })
        .unwrap();
        // The server first expires a snapshot, then invalidates a new snapshot after an external unlink.
        // Neither transition explicitly invalidates this client's reads.
        for watched in [Some(true), None, Some(true), None] {
            *state.watched.lock().unwrap() = watched;
            let home = client.clone().get_home(Default::default()).await.unwrap();
            assert_eq!(home[0].items[0].watched, watched);
            assert_eq!(home[0].items[0].progress_percent, watched.map(|_| 42));
            assert_eq!(
                client
                    .clone()
                    .get_item("item".into(), Default::default())
                    .await
                    .unwrap()
                    .watched,
                watched
            );
            for token in ["", "continuation"] {
                let page = Page {
                    size: 1,
                    token: token.into(),
                };
                assert_eq!(
                    client
                        .clone()
                        .list_library_items(SourceLibraryScope::default(), page.clone())
                        .await
                        .unwrap()
                        .items[0]
                        .watched,
                    watched
                );
                assert_eq!(
                    client
                        .clone()
                        .list_children("parent".into(), page.clone(), Default::default())
                        .await
                        .unwrap()
                        .items[0]
                        .watched,
                    watched
                );
                assert_eq!(
                    client
                        .clone()
                        .search(SearchRequest {
                            query: "item".into(),
                            page: Some(page),
                            ..Default::default()
                        })
                        .await
                        .unwrap()
                        .items[0]
                        .watched,
                    watched
                );
            }
            client.clone().get_catalog_info().await.unwrap();
            client
                .clone()
                .list_libraries("source".into(), Default::default())
                .await
                .unwrap();
        }
        assert_eq!(state.count("home"), 4);
        assert_eq!(state.count("item"), 4);
        for op in ["items", "children", "search"] {
            assert_eq!(state.count(op), 8);
        }
        assert_eq!(state.count("info"), 1);
        assert_eq!(state.count("libraries"), 1);
        let (a, b) = tokio::join!(
            client.clone().get_item("item".into(), Default::default()),
            client.clone().get_item("item".into(), Default::default())
        );
        assert_eq!(a.unwrap(), b.unwrap());
        assert_eq!(
            state.count("item"),
            5,
            "concurrent identical reads still share transport"
        );
        stop.send(()).unwrap();
        task.await.unwrap().unwrap();
    }
}
