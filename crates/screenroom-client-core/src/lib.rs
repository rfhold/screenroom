mod models;
use base64::{Engine, engine::general_purpose::STANDARD};
use futures::{
    FutureExt,
    future::{AbortHandle, Abortable, Shared},
};
pub use models::*;
use screenroom_proto::client::v1 as wire;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
};

#[cfg(not(target_arch = "wasm32"))]
uniffi::setup_scaffolding!();

#[derive(Clone, Debug, thiserror::Error)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Error))]
pub enum ClientError {
    #[error("invalid argument: {reason}")]
    InvalidArgument { reason: String },
    #[error("not found: {reason}")]
    NotFound { reason: String },
    #[error("unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("deadline exceeded")]
    DeadlineExceeded,
    #[error("cancelled")]
    Cancelled,
    #[error("resource exhausted: {reason}")]
    ResourceExhausted { reason: String },
    #[error("RPC {code}: {reason}")]
    Rpc { code: u32, reason: String },
    #[error("invalid response: {reason}")]
    InvalidResponse { reason: String },
}
impl From<tonic::Status> for ClientError {
    fn from(s: tonic::Status) -> Self {
        let reason = s.message().to_string();
        match s.code() {
            tonic::Code::InvalidArgument => Self::InvalidArgument { reason },
            tonic::Code::NotFound => Self::NotFound { reason },
            tonic::Code::Unavailable => Self::Unavailable { reason },
            tonic::Code::DeadlineExceeded => Self::DeadlineExceeded,
            tonic::Code::Cancelled => Self::Cancelled,
            tonic::Code::ResourceExhausted => Self::ResourceExhausted { reason },
            c => Self::Rpc {
                code: c as u32,
                reason,
            },
        }
    }
}
fn invalid(reason: impl Into<String>) -> ClientError {
    ClientError::InvalidArgument {
        reason: reason.into(),
    }
}
fn bridge<A: Serialize, B: DeserializeOwned>(a: A) -> Result<B, ClientError> {
    serde_json::to_value(a)
        .and_then(serde_json::from_value)
        .map_err(|e| ClientError::InvalidResponse {
            reason: e.to_string(),
        })
}

#[cfg(not(target_arch = "wasm32"))]
type Pending = Shared<futures::future::BoxFuture<'static, Result<Value, ClientError>>>;
#[cfg(target_arch = "wasm32")]
type Pending = Shared<futures::future::LocalBoxFuture<'static, Result<Value, ClientError>>>;
#[cfg(not(target_arch = "wasm32"))]
type Transport = tonic::transport::Channel;
#[cfg(target_arch = "wasm32")]
type Transport = tonic_web_wasm_client::Client;

#[derive(Default)]
struct Cache {
    values: VecDeque<(String, Value, usize)>,
    pending: HashMap<String, PendingEntry>,
    art: VecDeque<(String, Vec<u8>)>,
}
struct PendingEntry {
    future: Pending,
    abort: AbortHandle,
    waiters: usize,
}

struct PendingWaiter {
    client: Weak<Client>,
    key: String,
    future: Pending,
}
impl Drop for PendingWaiter {
    fn drop(&mut self) {
        let Some(client) = self.client.upgrade() else {
            return;
        };
        let Ok(mut cache) = client.cache.lock() else {
            return;
        };
        let removed = if let Some(entry) = cache.pending.get_mut(&self.key) {
            if !entry.future.ptr_eq(&self.future) {
                return;
            }
            entry.waiters -= 1;
            if entry.waiters == 0 {
                cache.pending.remove(&self.key)
            } else {
                None
            }
        } else {
            None
        };
        drop(cache);
        if let Some(entry) = removed {
            entry.abort.abort();
        }
    }
}
impl Cache {
    fn insert_art(&mut self, key: String, bytes: Vec<u8>) {
        if bytes.len() > 4 * 1024 * 1024 || self.art.iter().any(|(k, _)| k == &key) {
            return;
        }
        while self.art.len() >= 32
            || self.art.iter().map(|(_, v)| v.len()).sum::<usize>() + bytes.len() > 8 * 1024 * 1024
        {
            self.art.pop_front();
        }
        self.art.push_back((key, bytes));
    }
    fn insert_read(&mut self, key: String, value: Value) {
        let size = key.len() + value.to_string().len();
        if size > 2 * 1024 * 1024 || self.values.iter().any(|(k, _, _)| k == &key) {
            return;
        }
        while self.values.len() >= 64
            || self.values.iter().map(|(_, _, s)| s).sum::<usize>() + size > 2 * 1024 * 1024
        {
            self.values.pop_front();
        }
        self.values.push_back((key, value, size));
    }
}

#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Object))]
pub struct Client {
    config: ClientConfig,
    http: reqwest::Client,
    #[cfg(not(target_arch = "wasm32"))]
    channel: Arc<tokio::sync::OnceCell<Transport>>,
    generation: AtomicU64,
    cache: Mutex<Cache>,
}

#[cfg_attr(not(target_arch = "wasm32"), uniffi::export(async_runtime = "tokio"))]
impl Client {
    #[cfg_attr(not(target_arch = "wasm32"), uniffi::constructor)]
    pub fn new(config: ClientConfig) -> Result<Arc<Self>, ClientError> {
        let url = url::Url::parse(&config.endpoint).map_err(|e| invalid(e.to_string()))?;
        if !["http", "https"].contains(&url.scheme())
            || url.host_str().is_none()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(invalid(
                "endpoint must be HTTP(S) origin without credentials/path/query/fragment",
            ));
        }
        if config.endpoint.ends_with('/') {
            return Err(invalid("endpoint must not end in slash"));
        }
        if config.mock_profile.is_empty()
            || config.mock_profile.len() > 128
            || !config
                .mock_profile
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        {
            return Err(invalid(
                "explicit opaque mock_profile required (1–128 ASCII letters/digits/-_.)",
            ));
        }
        if !(1..=120_000).contains(&config.timeout_ms) {
            return Err(invalid("timeout_ms must be 1–120000"));
        }
        #[cfg(not(target_arch = "wasm32"))]
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| invalid(e.to_string()))?;
        #[cfg(target_arch = "wasm32")]
        let http = reqwest::Client::new();
        Ok(Arc::new(Self {
            config,
            http,
            #[cfg(not(target_arch = "wasm32"))]
            channel: Arc::new(tokio::sync::OnceCell::new()),
            generation: AtomicU64::new(0),
            cache: Mutex::new(Cache::default()),
        }))
    }
    pub fn cancel_all(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut c) = self.cache.lock() {
            for (_, entry) in c.pending.drain() {
                entry.abort.abort();
            }
        }
    }
    pub fn invalidate_reads(&self) {
        self.cancel_all();
        if let Ok(mut c) = self.cache.lock() {
            c.values.clear();
        }
    }
    pub async fn get_home(
        self: Arc<Self>,
        library_id: String,
    ) -> Result<Vec<HomeRow>, ClientError> {
        let response: wire::GetHomeResponse = self
            .read("home", wire::GetHomeRequest { library_id })
            .await?;
        bridge(response.rows)
    }
    pub async fn list_libraries(self: Arc<Self>) -> Result<Vec<MediaLibrary>, ClientError> {
        let response: wire::ListLibrariesResponse = self
            .read("libraries", wire::ListLibrariesRequest {})
            .await?;
        bridge(response.libraries)
    }
    pub async fn list_library_items(
        self: Arc<Self>,
        library_id: String,
        page: Page,
    ) -> Result<ItemPage, ClientError> {
        self.read(
            "items",
            wire::ListLibraryItemsRequest {
                library_id,
                page: Some(bridge(page)?),
            },
        )
        .await
    }
    pub async fn get_item(self: Arc<Self>, id: String) -> Result<Item, ClientError> {
        let response: wire::GetItemResponse =
            self.read("item", wire::GetItemRequest { id }).await?;
        bridge(response.item.ok_or_else(|| ClientError::InvalidResponse {
            reason: "missing item".into(),
        })?)
    }
    pub async fn get_catalog_info(self: Arc<Self>) -> Result<CatalogInfo, ClientError> {
        let response: wire::GetCatalogInfoResponse = self
            .read("catalog_info", wire::GetCatalogInfoRequest {})
            .await?;
        bridge(response.info.ok_or_else(|| ClientError::InvalidResponse {
            reason: "missing catalog info".into(),
        })?)
    }
    pub async fn list_children(
        self: Arc<Self>,
        parent_id: String,
        page: Page,
    ) -> Result<ItemPage, ClientError> {
        self.read(
            "children",
            wire::ListChildrenRequest {
                parent_id,
                page: Some(bridge(page)?),
            },
        )
        .await
    }
    pub async fn list_credits(
        self: Arc<Self>,
        item_id: String,
        page: Page,
    ) -> Result<CreditPage, ClientError> {
        self.read(
            "credits",
            wire::ListCreditsRequest {
                item_id,
                page: Some(bridge(page)?),
            },
        )
        .await
    }
    pub async fn search(self: Arc<Self>, request: SearchRequest) -> Result<ItemPage, ClientError> {
        self.read("search", bridge::<_, wire::SearchRequest>(request)?)
            .await
    }
    pub async fn get_search_capabilities(
        self: Arc<Self>,
    ) -> Result<SearchCapabilities, ClientError> {
        self.read("capabilities", wire::GetSearchCapabilitiesRequest {})
            .await
    }
    pub async fn get_preferences(self: Arc<Self>) -> Result<Preferences, ClientError> {
        // Preferences intentionally bypass the read cache so different clients sharing a profile see updates.
        let value = self
            .dedup(
                "preferences".into(),
                "preferences".into(),
                Value::Null,
                false,
            )
            .await?;
        bridge(
            value
                .get("preferences")
                .ok_or_else(|| ClientError::InvalidResponse {
                    reason: "missing preferences".into(),
                })?,
        )
    }
    pub async fn update_preferences(
        self: Arc<Self>,
        preferences: Preferences,
        update_mask: Vec<String>,
    ) -> Result<Preferences, ClientError> {
        static NEXT_WRITE: AtomicU64 = AtomicU64::new(0);
        let key = format!(
            "update_preferences:{}",
            NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
        );
        let value = self
            .clone()
            .dedup(
                key,
                "update_preferences".into(),
                serde_json::json!({"preferences":preferences,"paths":update_mask}),
                false,
            )
            .await?;
        self.lock()?.values.clear();
        bridge(
            value
                .get("preferences")
                .ok_or_else(|| ClientError::InvalidResponse {
                    reason: "missing preferences".into(),
                })?,
        )
    }
    pub async fn load_artwork(self: Arc<Self>, url: String) -> Result<Vec<u8>, ClientError> {
        let parsed = url::Url::parse(&url).map_err(|e| invalid(e.to_string()))?;
        let base = url::Url::parse(&self.config.endpoint).map_err(|e| invalid(e.to_string()))?;
        if parsed.origin() != base.origin()
            || !parsed.path().starts_with("/artwork/")
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(invalid(
                "artwork must be an /artwork/ URL on configured endpoint",
            ));
        }
        let key = format!("art:{url}");
        if let Some(bytes) = self
            .lock()?
            .art
            .iter()
            .find(|(k, _)| k == &url)
            .map(|(_, v)| v.clone())
        {
            return Ok(bytes);
        }
        let value = self
            .clone()
            .dedup(key, "art".into(), Value::String(url.clone()), false)
            .await?;
        let bytes = STANDARD
            .decode(value.as_str().ok_or_else(|| ClientError::InvalidResponse {
                reason: "missing artwork bytes".into(),
            })?)
            .map_err(|e| ClientError::InvalidResponse {
                reason: e.to_string(),
            })?;
        self.lock()?.insert_art(url, bytes.clone());
        Ok(bytes)
    }
}

impl Client {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Cache>, ClientError> {
        self.cache.lock().map_err(|_| ClientError::InvalidResponse {
            reason: "cache lock poisoned".into(),
        })
    }
    async fn transport(&self) -> Result<Transport, ClientError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.channel
                .get_or_try_init(|| async {
                    tonic::transport::Endpoint::from_shared(self.config.endpoint.clone())
                        .map_err(|e| invalid(e.to_string()))?
                        .connect_timeout(std::time::Duration::from_millis(
                            self.config.timeout_ms.into(),
                        ))
                        .connect()
                        .await
                        .map_err(|e| ClientError::Unavailable {
                            reason: e.to_string(),
                        })
                })
                .await
                .cloned()
        }
        #[cfg(target_arch = "wasm32")]
        {
            Ok(tonic_web_wasm_client::Client::new(
                self.config.endpoint.clone(),
            ))
        }
    }
    fn request<T>(&self, body: T) -> Result<tonic::Request<T>, ClientError> {
        let mut r = tonic::Request::new(body);
        r.metadata_mut().insert(
            "x-screenroom-mock-profile",
            self.config
                .mock_profile
                .parse()
                .map_err(|_| invalid("invalid profile header"))?,
        );
        Ok(r)
    }
    async fn read<Q: Serialize, R: DeserializeOwned>(
        self: Arc<Self>,
        op: &'static str,
        request: Q,
    ) -> Result<R, ClientError> {
        let value = serde_json::to_value(request).map_err(|e| invalid(e.to_string()))?;
        let key = format!("{op}:{value}");
        if let Some(v) = self
            .lock()?
            .values
            .iter()
            .find(|(k, _, _)| k == &key)
            .map(|(_, v, _)| v.clone())
        {
            return bridge(v);
        }
        bridge(self.dedup(key, op.into(), value, true).await?)
    }
    async fn dedup(
        self: Arc<Self>,
        key: String,
        op: String,
        value: Value,
        cache: bool,
    ) -> Result<Value, ClientError> {
        let generation = self.generation.load(Ordering::SeqCst);
        let future = {
            let mut c = self.lock()?;
            if self.generation.load(Ordering::SeqCst) != generation {
                return Err(ClientError::Cancelled);
            }
            if let Some(entry) = c.pending.get_mut(&key) {
                entry.waiters += 1;
                entry.future.clone()
            } else {
                if c.pending.len() >= 32 {
                    return Err(ClientError::ResourceExhausted {
                        reason: "32 in-flight requests already active".into(),
                    });
                }
                let (handle, registration) = AbortHandle::new_pair();
                let client = Self {
                    config: self.config.clone(),
                    http: self.http.clone(),
                    #[cfg(not(target_arch = "wasm32"))]
                    channel: self.channel.clone(),
                    generation: AtomicU64::new(0),
                    cache: Mutex::default(),
                };
                let f = async move {
                    Abortable::new(client.call(&op, value), registration)
                        .await
                        .map_err(|_| ClientError::Cancelled)?
                };
                #[cfg(not(target_arch = "wasm32"))]
                let f = f.boxed().shared();
                #[cfg(target_arch = "wasm32")]
                let f = f.boxed_local().shared();
                c.pending.insert(
                    key.clone(),
                    PendingEntry {
                        future: f.clone(),
                        abort: handle,
                        waiters: 1,
                    },
                );
                f
            }
        };
        let _waiter = PendingWaiter {
            client: Arc::downgrade(&self),
            key: key.clone(),
            future: future.clone(),
        };
        let result = future.clone().await;
        let mut c = self.lock()?;
        if c.pending
            .get(&key)
            .is_some_and(|entry| entry.future.ptr_eq(&future))
        {
            c.pending.remove(&key);
        }
        if self.generation.load(Ordering::SeqCst) != generation {
            return Err(ClientError::Cancelled);
        }
        if cache {
            if let Ok(v) = &result {
                c.insert_read(key, v.clone());
            }
        }
        result
    }
    async fn call(&self, op: &str, value: Value) -> Result<Value, ClientError> {
        timeout(self.config.timeout_ms, self.network(op, value)).await
    }
    async fn network(&self, op: &str, value: Value) -> Result<Value, ClientError> {
        if op == "art" {
            let url = value.as_str().ok_or_else(|| invalid("missing art URL"))?;
            let response =
                self.http
                    .get(url)
                    .send()
                    .await
                    .map_err(|e| ClientError::Unavailable {
                        reason: e.to_string(),
                    })?;
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(ClientError::NotFound {
                    reason: "artwork not found".into(),
                });
            }
            if !response.status().is_success() {
                return Err(ClientError::Unavailable {
                    reason: format!("artwork HTTP {}", response.status()),
                });
            }
            if response
                .content_length()
                .is_some_and(|n| n > 4 * 1024 * 1024)
            {
                return Err(ClientError::ResourceExhausted {
                    reason: "artwork exceeds 4 MiB".into(),
                });
            }
            let mime = response
                .headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok())
                .unwrap_or("");
            if !["image/jpeg", "image/png", "image/svg+xml"].contains(&mime) {
                return Err(ClientError::InvalidResponse {
                    reason: "unsupported artwork MIME".into(),
                });
            }
            let mut stream = response.bytes_stream();
            let mut bytes = Vec::new();
            use futures::StreamExt;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| ClientError::Unavailable {
                    reason: e.to_string(),
                })?;
                if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
                    return Err(ClientError::ResourceExhausted {
                        reason: "artwork exceeds 4 MiB".into(),
                    });
                }
                bytes.extend_from_slice(&chunk);
            }
            return Ok(Value::String(STANDARD.encode(bytes)));
        }
        let transport = self.transport().await?;
        macro_rules! rpc {
            ($module:ident,$client:ident,$method:ident,$req:ty) => {{
                let body: $req = bridge(value)?;
                let response = wire::$module::$client::new(transport)
                    .$method(self.request(body)?)
                    .await?;
                bridge(response.into_inner())
            }};
        }
        match op {
            "catalog_info" => rpc!(
                catalog_service_client,
                CatalogServiceClient,
                get_catalog_info,
                wire::GetCatalogInfoRequest
            ),
            "home" => rpc!(
                browse_service_client,
                BrowseServiceClient,
                get_home,
                wire::GetHomeRequest
            ),
            "libraries" => rpc!(
                browse_service_client,
                BrowseServiceClient,
                list_libraries,
                wire::ListLibrariesRequest
            ),
            "items" => rpc!(
                browse_service_client,
                BrowseServiceClient,
                list_library_items,
                wire::ListLibraryItemsRequest
            ),
            "item" => rpc!(
                catalog_service_client,
                CatalogServiceClient,
                get_item,
                wire::GetItemRequest
            ),
            "children" => rpc!(
                catalog_service_client,
                CatalogServiceClient,
                list_children,
                wire::ListChildrenRequest
            ),
            "credits" => rpc!(
                catalog_service_client,
                CatalogServiceClient,
                list_credits,
                wire::ListCreditsRequest
            ),
            "search" => rpc!(
                search_service_client,
                SearchServiceClient,
                search,
                wire::SearchRequest
            ),
            "capabilities" => rpc!(
                search_service_client,
                SearchServiceClient,
                get_search_capabilities,
                wire::GetSearchCapabilitiesRequest
            ),
            "preferences" => {
                let response =
                    wire::preferences_service_client::PreferencesServiceClient::new(transport)
                        .get_preferences(self.request(wire::GetPreferencesRequest {})?)
                        .await?;
                bridge(response.into_inner())
            }
            "update_preferences" => {
                let body = wire::UpdatePreferencesRequest {
                    preferences: Some(bridge(value["preferences"].clone())?),
                    update_mask: Some(prost_types::FieldMask {
                        paths: bridge(value["paths"].clone())?,
                    }),
                };
                let response =
                    wire::preferences_service_client::PreferencesServiceClient::new(transport)
                        .update_preferences(self.request(body)?)
                        .await?;
                bridge(response.into_inner())
            }
            _ => Err(invalid("unknown operation")),
        }
    }
}
async fn timeout<T>(
    ms: u32,
    f: impl std::future::Future<Output = Result<T, ClientError>>,
) -> Result<T, ClientError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        tokio::time::timeout(std::time::Duration::from_millis(ms.into()), f)
            .await
            .map_err(|_| ClientError::DeadlineExceeded)?
    }
    #[cfg(target_arch = "wasm32")]
    {
        let deadline = gloo_timers::future::TimeoutFuture::new(ms);
        futures::pin_mut!(f, deadline);
        match futures::future::select(f, deadline).await {
            futures::future::Either::Left((r, _)) => r,
            _ => Err(ClientError::DeadlineExceeded),
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    async fn stalled_client() -> (Arc<Client>, tokio::net::TcpListener) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = Client::new(ClientConfig {
            endpoint: format!("http://{}", listener.local_addr().unwrap()),
            mock_profile: "ownership".into(),
            timeout_ms: 25,
        })
        .unwrap();
        (client, listener)
    }
    #[test]
    fn dropped_last_waiters_release_all_slots_without_cancel_all() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let (client, _listener) = stalled_client().await;
            let mut requests = (0..32)
                .map(|n| Box::pin(client.clone().get_item(n.to_string())))
                .collect::<Vec<_>>();
            for request in &mut requests {
                assert!(futures::poll!(request.as_mut()).is_pending());
            }
            assert_eq!(client.lock().unwrap().pending.len(), 32);
            drop(requests);
            assert!(client.lock().unwrap().pending.is_empty());
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            assert!(matches!(
                client.clone().get_item("new-request".into()).await,
                Err(ClientError::DeadlineExceeded)
            ));
            assert!(client.lock().unwrap().pending.is_empty());
            assert_eq!(client.generation.load(Ordering::SeqCst), 0);
        });
    }
    #[test]
    fn dropping_one_dedup_waiter_preserves_the_other() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let (client, _listener) = stalled_client().await;
            let mut first = Box::pin(client.clone().get_item("shared".into()));
            let mut second = Box::pin(client.clone().get_item("shared".into()));
            assert!(futures::poll!(first.as_mut()).is_pending());
            assert!(futures::poll!(second.as_mut()).is_pending());
            assert_eq!(
                client
                    .lock()
                    .unwrap()
                    .pending
                    .values()
                    .next()
                    .unwrap()
                    .waiters,
                2
            );
            drop(first);
            assert_eq!(
                client
                    .lock()
                    .unwrap()
                    .pending
                    .values()
                    .next()
                    .unwrap()
                    .waiters,
                1
            );
            assert!(matches!(second.await, Err(ClientError::DeadlineExceeded)));
            assert!(client.lock().unwrap().pending.is_empty());
            assert_eq!(client.generation.load(Ordering::SeqCst), 0);
        });
    }
    #[test]
    fn old_generation_waiter_cannot_remove_replacement_request() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let (client, _listener) = stalled_client().await;
            let mut old = Box::pin(client.clone().get_item("shared".into()));
            assert!(futures::poll!(old.as_mut()).is_pending());
            client.cancel_all();
            assert_eq!(client.generation.load(Ordering::SeqCst), 1);
            let mut replacement = Box::pin(client.clone().get_item("shared".into()));
            assert!(futures::poll!(replacement.as_mut()).is_pending());
            assert!(matches!(old.await, Err(ClientError::Cancelled)));
            assert_eq!(client.lock().unwrap().pending.len(), 1);
            assert_eq!(
                client
                    .lock()
                    .unwrap()
                    .pending
                    .values()
                    .next()
                    .unwrap()
                    .waiters,
                1
            );
            drop(replacement);
            assert!(client.lock().unwrap().pending.is_empty());
        });
    }
    #[test]
    fn cache_count_and_byte_limits() {
        let mut cache = Cache::default();
        for n in 0..40 {
            cache.insert_art(n.to_string(), vec![0; 100]);
        }
        assert_eq!(cache.art.len(), 32);
        for n in 40..45 {
            cache.insert_art(n.to_string(), vec![0; 3 * 1024 * 1024]);
        }
        assert_eq!(cache.art.len(), 2);
        assert!(cache.art.iter().map(|(_, v)| v.len()).sum::<usize>() <= 8 * 1024 * 1024);
        for n in 0..70 {
            cache.insert_read(n.to_string(), Value::Null);
        }
        assert_eq!(cache.values.len(), 64);
        cache.insert_read("large".into(), Value::String("x".repeat(1024 * 1024)));
        cache.insert_read("large2".into(), Value::String("x".repeat(1024 * 1024)));
        assert_eq!(cache.values.len(), 1);
        cache.insert_read(
            "oversize".into(),
            Value::String("x".repeat(3 * 1024 * 1024)),
        );
        assert_eq!(cache.values.len(), 1);
    }
}
