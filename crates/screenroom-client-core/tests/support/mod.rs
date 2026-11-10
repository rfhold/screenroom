use screenroom_proto::catalog::v1 as wire;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tonic::{Request, Response, Status};

pub type Counts = Arc<Mutex<HashMap<(String, String), usize>>>;

/// Transport double only: tiny payloads exercise cancellation and cache ownership.
#[derive(Clone, Default)]
pub struct TransportDouble {
    pub counts: Counts,
}
impl TransportDouble {
    async fn record<T>(&self, request: &Request<T>, operation: &str) {
        let user = request
            .metadata()
            .get("x-screenroom-user-id")
            .map(|v| v.to_str().unwrap())
            .unwrap_or("");
        *self
            .counts
            .lock()
            .unwrap()
            .entry((user.to_owned(), operation.to_owned()))
            .or_default() += 1;
        tokio::time::sleep(std::time::Duration::from_millis(if operation == "home" {
            5
        } else {
            800
        }))
        .await;
    }
}
#[tonic::async_trait]
impl wire::catalog_service_server::CatalogService for TransportDouble {
    async fn get_catalog_info(
        &self,
        r: Request<wire::GetCatalogInfoRequest>,
    ) -> Result<Response<wire::GetCatalogInfoResponse>, Status> {
        self.record(&r, "info").await;
        Ok(Response::new(wire::GetCatalogInfoResponse {
            info: Some(wire::CatalogInfo::default()),
        }))
    }
    async fn get_item(
        &self,
        r: Request<wire::GetItemRequest>,
    ) -> Result<Response<wire::GetItemResponse>, Status> {
        self.record(&r, "item").await;
        if r.get_ref().id != "transport-value" {
            return Err(Status::not_found("test-only missing item"));
        }
        Ok(Response::new(wire::GetItemResponse {
            item: Some(wire::Item {
                id: "transport-value".into(),
                title: "transport-value".into(),
                ..Default::default()
            }),
        }))
    }
    async fn list_children(
        &self,
        _: Request<wire::ListChildrenRequest>,
    ) -> Result<Response<wire::ListChildrenResponse>, Status> {
        Err(Status::unimplemented("not a fixture API"))
    }
    async fn list_credits(
        &self,
        _: Request<wire::ListCreditsRequest>,
    ) -> Result<Response<wire::ListCreditsResponse>, Status> {
        Err(Status::unimplemented("not a fixture API"))
    }
    async fn list_artwork(
        &self,
        _: Request<wire::ListArtworkRequest>,
    ) -> Result<Response<wire::ListArtworkResponse>, Status> {
        Err(Status::unimplemented("not a fixture API"))
    }
}
#[tonic::async_trait]
impl wire::browse_service_server::BrowseService for TransportDouble {
    async fn get_home(
        &self,
        r: Request<wire::GetHomeRequest>,
    ) -> Result<Response<wire::GetHomeResponse>, Status> {
        self.record(&r, "home").await;
        Ok(Response::new(wire::GetHomeResponse::default()))
    }
    async fn list_libraries(
        &self,
        r: Request<wire::ListLibrariesRequest>,
    ) -> Result<Response<wire::ListLibrariesResponse>, Status> {
        self.record(&r, "libraries").await;
        Ok(Response::new(wire::ListLibrariesResponse::default()))
    }
    async fn list_library_items(
        &self,
        _: Request<wire::ListLibraryItemsRequest>,
    ) -> Result<Response<wire::ListLibraryItemsResponse>, Status> {
        Err(Status::unimplemented("not a fixture API"))
    }
}
pub async fn start() -> (String, Counts, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let service = TransportDouble::default();
    let counts = service.counts.clone();
    let incoming = futures::stream::unfold(listener, |listener| async {
        Some((listener.accept().await.map(|(stream, _)| stream), listener))
    });
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(wire::catalog_service_server::CatalogServiceServer::new(
                service.clone(),
            ))
            .add_service(wire::browse_service_server::BrowseServiceServer::new(
                service,
            ))
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });
    (endpoint, counts, task)
}
