mod models;
mod operator;
mod operator_models;
mod playback;
pub use operator::*;
pub use operator_models::*;
#[cfg(target_arch = "wasm32")]
mod operator_wasm;
use base64::{Engine, engine::general_purpose::STANDARD};
use futures::{
    FutureExt,
    future::{AbortHandle, Abortable, Shared},
};
pub use models::*;
mod wire {
    pub use screenroom_proto::catalog::v1::*;
    pub use screenroom_proto::user::v1::{
        ClearJellyfinLinkRequest, ClearJellyfinLinkResponse, GetCurrentUserRequest,
        GetCurrentUserResponse, GetJellyfinLinkRequest, GetJellyfinLinkResponse,
        ListJellyfinProfilesRequest, ListJellyfinSourcesRequest, SetJellyfinLinkRequest,
        SetJellyfinLinkResponse, user_service_client,
    };
}
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

#[cfg(not(target_arch = "wasm32"))]
uniffi::setup_scaffolding!();

#[derive(Clone, Debug, thiserror::Error)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Error))]
pub enum ClientError {
    #[error("playback: {code:?}")]
    Playback { code: PlaybackErrorCode },
    #[error("playback outcome unknown; reconcile before another mutation")]
    PlaybackUncertain,
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
fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
fn valid_artwork_path(path: &str) -> bool {
    let parts = path.split('/').collect::<Vec<_>>();
    parts.len() == 6
        && parts[1] == "artwork"
        && valid_uuid(parts[2])
        && parts[3] == "revisions"
        && parts[4].bytes().all(|b| b.is_ascii_digit())
        && parts[4].parse::<u64>().is_ok_and(|n| n > 0)
        && ["native", "original"].contains(&parts[5])
}

/// Platform-owned settings: callers persist these locally, never through an RPC.
#[cfg_attr(not(target_arch = "wasm32"), uniffi::export)]
pub fn default_settings() -> ClientSettings {
    ClientSettings {
        scope: Some(SourceLibraryScope::default()),
        source: "all".into(),
        types: Some(SearchTypes {
            movie: true,
            series: true,
            episode: true,
        }),
        methods: Some(SearchMethods {
            text: true,
            fuzzy: true,
            semantic: true,
            title_only: false,
        }),
    }
}
#[cfg_attr(not(target_arch = "wasm32"), uniffi::export)]
pub fn validate_settings(settings: ClientSettings) -> Result<ClientSettings, ClientError> {
    if !["all", "jellyfin"].contains(&settings.source.as_str()) {
        return Err(invalid("source must be all or jellyfin"));
    }
    if settings
        .types
        .as_ref()
        .is_some_and(|t| !t.movie && !t.series && !t.episode)
    {
        return Err(invalid("at least one search type required"));
    }
    if settings
        .methods
        .as_ref()
        .is_some_and(|m| !m.text && !m.fuzzy && (!m.semantic || m.title_only))
    {
        return Err(invalid("at least one search engine required"));
    }
    Ok(settings)
}
/// Load-only migration of retired source preferences; saves use strict validation.
#[cfg_attr(not(target_arch = "wasm32"), uniffi::export)]
pub fn normalize_saved_settings(
    mut settings: ClientSettings,
) -> Result<ClientSettings, ClientError> {
    if settings.source == "tmdb" {
        settings.source = "all".into();
    }
    validate_settings(settings)
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
    cancellation: AtomicU64,
    playback_generation: AtomicU64,
    cache: Mutex<Cache>,
    mutation: futures::lock::Mutex<()>,
    mutation_waiters: AtomicUsize,
    #[cfg(all(test, not(target_arch = "wasm32")))]
    artwork_completion_hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
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
        if !config.user_id.is_empty() && !valid_uuid(&config.user_id) {
            return Err(invalid(
                "user_id must be empty or a canonical local UUID; this selector is not authentication",
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
            cancellation: AtomicU64::new(0),
            playback_generation: AtomicU64::new(0),
            cache: Mutex::new(Cache::default()),
            mutation: futures::lock::Mutex::new(()),
            mutation_waiters: AtomicUsize::new(0),
            #[cfg(all(test, not(target_arch = "wasm32")))]
            artwork_completion_hook: Mutex::new(None),
        }))
    }
    pub fn cancel_all(&self) {
        self.cancellation.fetch_add(1, Ordering::SeqCst);
        let pending = if let Ok(mut c) = self.cache.lock() {
            // Cancellation and cache completion share this linearization point.
            self.generation.fetch_add(1, Ordering::SeqCst);
            c.pending
                .drain()
                .map(|(_, entry)| entry.abort)
                .collect::<Vec<_>>()
        } else {
            return;
        };
        for abort in pending {
            abort.abort();
        }
    }
    /// Invalidates cached nonpersonal reads and pending reads, including personal item reads.
    /// Profile mutations use this boundary; completed item reads are never retained.
    pub fn invalidate_reads(&self) {
        if let Ok(mut c) = self.cache.lock() {
            self.generation.fetch_add(1, Ordering::SeqCst);
            c.values.clear();
            for (_, entry) in c.pending.drain() {
                entry.abort.abort();
            }
        }
    }
    pub async fn list_jellyfin_sources(
        self: Arc<Self>,
        page: Page,
    ) -> Result<JellyfinSourcePage, ClientError> {
        self.require_user()?;
        self.read(
            "jellyfin_sources",
            wire::ListJellyfinSourcesRequest {
                page: Some(bridge(page)?),
            },
        )
        .await
    }
    pub async fn list_jellyfin_profiles(
        self: Arc<Self>,
        source_id: String,
        page: Page,
    ) -> Result<JellyfinProfilePage, ClientError> {
        self.require_user()?;
        self.read(
            "jellyfin_profiles",
            wire::ListJellyfinProfilesRequest {
                source_id,
                page: Some(bridge(page)?),
            },
        )
        .await
    }
    pub async fn get_jellyfin_link(
        self: Arc<Self>,
        source_id: String,
    ) -> Result<Option<JellyfinLink>, ClientError> {
        self.require_user()?;
        let response: wire::GetJellyfinLinkResponse = self
            .read("jellyfin_link", wire::GetJellyfinLinkRequest { source_id })
            .await?;
        bridge(response.link)
    }
    pub async fn set_jellyfin_link(
        self: Arc<Self>,
        source_id: String,
        jellyfin_user_id: String,
    ) -> Result<JellyfinLink, ClientError> {
        self.cancel_playback_requests();
        let response: wire::SetJellyfinLinkResponse = self
            .mutate(
                "set_jellyfin_link",
                wire::SetJellyfinLinkRequest {
                    source_id,
                    jellyfin_user_id,
                },
            )
            .await?;
        bridge(response.link.ok_or_else(|| ClientError::InvalidResponse {
            reason: "missing link".into(),
        })?)
    }
    pub async fn clear_jellyfin_link(
        self: Arc<Self>,
        source_id: String,
    ) -> Result<String, ClientError> {
        self.cancel_playback_requests();
        let response: wire::ClearJellyfinLinkResponse = self
            .mutate(
                "clear_jellyfin_link",
                wire::ClearJellyfinLinkRequest { source_id },
            )
            .await?;
        Ok(response.source_id)
    }
    pub async fn get_home(
        self: Arc<Self>,
        scope: SourceLibraryScope,
    ) -> Result<Vec<HomeRow>, ClientError> {
        let response: wire::GetHomeResponse = self
            .read(
                "home",
                wire::GetHomeRequest {
                    library_id: scope.library_id,
                    source_id: scope.source_id,
                },
            )
            .await?;
        bridge(response.rows)
    }
    pub async fn list_libraries(
        self: Arc<Self>,
        source_id: String,
        page: Page,
    ) -> Result<LibraryPage, ClientError> {
        self.read(
            "libraries",
            wire::ListLibrariesRequest {
                source_id,
                page: Some(bridge(page)?),
            },
        )
        .await
    }
    pub async fn list_library_items(
        self: Arc<Self>,
        scope: SourceLibraryScope,
        page: Page,
    ) -> Result<ItemPage, ClientError> {
        self.read(
            "items",
            wire::ListLibraryItemsRequest {
                library_id: scope.library_id,
                source_id: scope.source_id,
                page: Some(bridge(page)?),
            },
        )
        .await
    }
    pub async fn get_item(
        self: Arc<Self>,
        id: String,
        scope: SourceLibraryScope,
    ) -> Result<Item, ClientError> {
        let response: wire::GetItemResponse = self
            .read(
                "item",
                wire::GetItemRequest {
                    id,
                    scope: Some(bridge(scope)?),
                },
            )
            .await?;
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
        scope: SourceLibraryScope,
    ) -> Result<ItemPage, ClientError> {
        self.read(
            "children",
            wire::ListChildrenRequest {
                parent_id,
                scope: Some(bridge(scope)?),
                page: Some(bridge(page)?),
            },
        )
        .await
    }
    pub async fn list_credits(
        self: Arc<Self>,
        item_id: String,
        page: Page,
        scope: SourceLibraryScope,
    ) -> Result<CreditPage, ClientError> {
        self.read(
            "credits",
            wire::ListCreditsRequest {
                item_id,
                scope: Some(bridge(scope)?),
                page: Some(bridge(page)?),
            },
        )
        .await
    }
    pub async fn search(
        self: Arc<Self>,
        request: SearchRequest,
    ) -> Result<SearchPage, ClientError> {
        self.read("search", bridge::<_, wire::SearchRequest>(request)?)
            .await
    }
    pub async fn get_search_capabilities(
        self: Arc<Self>,
    ) -> Result<SearchCapabilities, ClientError> {
        self.read("capabilities", wire::GetSearchCapabilitiesRequest {})
            .await
    }
    pub async fn get_current_user(self: Arc<Self>) -> Result<User, ClientError> {
        if self.config.user_id.is_empty() {
            return Err(invalid(
                "get_current_user requires an explicit local user_id",
            ));
        }
        let response: wire::GetCurrentUserResponse = self
            .read("current_user", wire::GetCurrentUserRequest {})
            .await?;
        bridge(response.user.ok_or_else(|| ClientError::InvalidResponse {
            reason: "missing user".into(),
        })?)
    }
    pub async fn list_artwork(
        self: Arc<Self>,
        scope: SourceLibraryScope,
        page: Page,
    ) -> Result<ArtworkPage, ClientError> {
        self.read(
            "artwork",
            wire::ListArtworkRequest {
                scope: Some(bridge(scope)?),
                page: Some(bridge(page)?),
            },
        )
        .await
    }
    pub async fn load_artwork(self: Arc<Self>, url: String) -> Result<Vec<u8>, ClientError> {
        let parsed = url::Url::parse(&url).map_err(|e| invalid(e.to_string()))?;
        let base = url::Url::parse(&self.config.endpoint).map_err(|e| invalid(e.to_string()))?;
        if parsed.origin() != base.origin()
            || !valid_artwork_path(parsed.path())
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
        let generation = {
            let cache = self.lock()?;
            if let Some((_, bytes)) = cache.art.iter().find(|(k, _)| k == &url) {
                return Ok(bytes.clone());
            }
            self.generation.load(Ordering::SeqCst)
        };
        let value = self
            .clone()
            .dedup(key, "art".into(), Value::String(url.clone()), false)
            .await?;
        #[cfg(all(test, not(target_arch = "wasm32")))]
        if let Some(hook) = self.artwork_completion_hook.lock().unwrap().take() {
            hook();
        }
        let bytes = STANDARD
            .decode(value.as_str().ok_or_else(|| ClientError::InvalidResponse {
                reason: "missing artwork bytes".into(),
            })?)
            .map_err(|e| ClientError::InvalidResponse {
                reason: e.to_string(),
            })?;
        let mut cache = self.lock()?;
        if self.generation.load(Ordering::SeqCst) != generation {
            return Err(ClientError::Cancelled);
        }
        cache.insert_art(url, bytes.clone());
        Ok(bytes)
    }
}

impl Client {
    fn require_user(&self) -> Result<(), ClientError> {
        if self.config.user_id.is_empty() {
            return Err(invalid("requires an explicit local user_id"));
        }
        Ok(())
    }
    async fn mutate<Q: Serialize, R: DeserializeOwned>(
        self: Arc<Self>,
        op: &'static str,
        request: Q,
    ) -> Result<R, ClientError> {
        self.require_user()?;
        if self.mutation_waiters.fetch_add(1, Ordering::SeqCst) >= 32 {
            self.mutation_waiters.fetch_sub(1, Ordering::SeqCst);
            return Err(ClientError::ResourceExhausted {
                reason: "32 mutation requests already active".into(),
            });
        }
        let _admission = MutationAdmission(self.clone());
        let generation = self.cancellation.load(Ordering::SeqCst);
        timeout(self.config.timeout_ms, async {
            let _serial = self.mutation.lock().await;
            if self.cancellation.load(Ordering::SeqCst) != generation {
                return Err(ClientError::Cancelled);
            }
            self.invalidate_reads();
            let _barrier = MutationBarrier(self.clone());
            let key = format!("mutation:{}", self.generation.load(Ordering::SeqCst));
            bridge(
                self.clone()
                    .dedup(
                        key,
                        op.into(),
                        serde_json::to_value(request).map_err(|e| invalid(e.to_string()))?,
                        false,
                    )
                    .await?,
            )
        })
        .await
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Cache>, ClientError> {
        self.cache.lock().map_err(|_| ClientError::InvalidResponse {
            reason: "cache lock poisoned".into(),
        })
    }
    async fn transport(&self) -> Result<Transport, ClientError> {
        self.transport_budget(self.config.timeout_ms).await
    }
    async fn transport_budget(&self, budget: u32) -> Result<Transport, ClientError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.channel
                .get_or_try_init(|| async {
                    tonic::transport::Endpoint::from_shared(self.config.endpoint.clone())
                        .map_err(|e| invalid(e.to_string()))?
                        .connect_timeout(std::time::Duration::from_millis(budget.into()))
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
            let _ = budget;
            Ok(tonic_web_wasm_client::Client::new(
                self.config.endpoint.clone(),
            ))
        }
    }
    fn request<T>(&self, body: T) -> Result<tonic::Request<T>, ClientError> {
        let mut r = tonic::Request::new(body);
        if !self.config.user_id.is_empty() {
            r.metadata_mut().insert(
                "x-screenroom-user-id",
                self.config
                    .user_id
                    .parse()
                    .map_err(|_| invalid("invalid local user header"))?,
            );
        }
        Ok(r)
    }
    async fn read<Q: Serialize, R: DeserializeOwned>(
        self: Arc<Self>,
        op: &'static str,
        request: Q,
    ) -> Result<R, ClientError> {
        let value = serde_json::to_value(request).map_err(|e| invalid(e.to_string()))?;
        let key = format!(
            "{}:{}:{op}:{value}",
            self.config.endpoint, self.config.user_id
        );
        // Item responses carry personal observations without a validity deadline or revision.
        let retain = !matches!(op, "home" | "items" | "item" | "children" | "search");
        if retain
            && let Some(v) = self
                .lock()?
                .values
                .iter()
                .find(|(k, _, _)| k == &key)
                .map(|(_, v, _)| v.clone())
        {
            return bridge(v);
        }
        bridge(self.dedup(key, op.into(), value, retain).await?)
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
                    cancellation: AtomicU64::new(0),
                    playback_generation: AtomicU64::new(0),
                    cache: Mutex::default(),
                    mutation: futures::lock::Mutex::new(()),
                    mutation_waiters: AtomicUsize::new(0),
                    #[cfg(all(test, not(target_arch = "wasm32")))]
                    artwork_completion_hook: Mutex::new(None),
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
        if cache && let Ok(v) = &result {
            c.insert_read(key, v.clone());
        }
        result
    }
    async fn call(&self, op: &str, value: Value) -> Result<Value, ClientError> {
        let budget = playback::budget(op).unwrap_or(self.config.timeout_ms);
        timeout(budget, self.network(op, value)).await
    }
    async fn network(&self, op: &str, value: Value) -> Result<Value, ClientError> {
        if op.starts_with("playback_") {
            return self.playback_network(op, value).await;
        }
        if op == "art" {
            let url = value.as_str().ok_or_else(|| invalid("missing art URL"))?;
            #[cfg(target_arch = "wasm32")]
            return Ok(Value::String(
                STANDARD.encode(wasm::fetch_artwork(url).await?),
            ));
            #[cfg(not(target_arch = "wasm32"))]
            {
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
            "jellyfin_sources" => rpc!(
                user_service_client,
                UserServiceClient,
                list_jellyfin_sources,
                wire::ListJellyfinSourcesRequest
            ),
            "jellyfin_profiles" => rpc!(
                user_service_client,
                UserServiceClient,
                list_jellyfin_profiles,
                wire::ListJellyfinProfilesRequest
            ),
            "jellyfin_link" => rpc!(
                user_service_client,
                UserServiceClient,
                get_jellyfin_link,
                wire::GetJellyfinLinkRequest
            ),
            "set_jellyfin_link" => rpc!(
                user_service_client,
                UserServiceClient,
                set_jellyfin_link,
                wire::SetJellyfinLinkRequest
            ),
            "clear_jellyfin_link" => rpc!(
                user_service_client,
                UserServiceClient,
                clear_jellyfin_link,
                wire::ClearJellyfinLinkRequest
            ),
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
            "artwork" => rpc!(
                catalog_service_client,
                CatalogServiceClient,
                list_artwork,
                wire::ListArtworkRequest
            ),
            "current_user" => rpc!(
                user_service_client,
                UserServiceClient,
                get_current_user,
                wire::GetCurrentUserRequest
            ),
            _ => Err(invalid("unknown operation")),
        }
    }
}
struct MutationBarrier(Arc<Client>);
struct MutationAdmission(Arc<Client>);
impl Drop for MutationAdmission {
    fn drop(&mut self) {
        self.0.mutation_waiters.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Drop for MutationBarrier {
    fn drop(&mut self) {
        self.0.invalidate_reads();
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
    #[test]
    fn cancellation_after_artwork_transport_cannot_populate_cache() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let client = Client::new(ClientConfig {
                endpoint: endpoint.clone(), user_id: String::new(), timeout_ms: 1000,
            }).unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                assert!(stream.read(&mut request).await.unwrap() > 0);
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc").await.unwrap();
            });
            let completed = Arc::new(std::sync::Barrier::new(2));
            let resume = Arc::new(std::sync::Barrier::new(2));
            let (at_completion, continue_completion) = (completed.clone(), resume.clone());
            *client.artwork_completion_hook.lock().unwrap() = Some(Box::new(move || {
                at_completion.wait();
                continue_completion.wait();
            }));
            let operation = tokio::spawn(client.clone().load_artwork(format!("{endpoint}/artwork/00000000-0000-0000-0000-000000000001/revisions/1/native")));
            completed.wait(); // The real transport and dedup have completed, insertion has not.
            client.cancel_all();
            resume.wait();
            assert!(matches!(operation.await.unwrap(), Err(ClientError::Cancelled)));
            assert!(client.lock().unwrap().art.is_empty());
            assert!(client.lock().unwrap().pending.is_empty());
            server.await.unwrap();
        });
    }
    async fn stalled_client() -> (Arc<Client>, tokio::net::TcpListener) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = Client::new(ClientConfig {
            endpoint: format!("http://{}", listener.local_addr().unwrap()),
            user_id: String::new(),
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
                .map(|n| {
                    Box::pin(
                        client
                            .clone()
                            .get_item(n.to_string(), SourceLibraryScope::default()),
                    )
                })
                .collect::<Vec<_>>();
            for request in &mut requests {
                assert!(futures::poll!(request.as_mut()).is_pending());
            }
            assert_eq!(client.lock().unwrap().pending.len(), 32);
            drop(requests);
            assert!(client.lock().unwrap().pending.is_empty());
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            assert!(matches!(
                client
                    .clone()
                    .get_item("new-request".into(), SourceLibraryScope::default())
                    .await,
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
            let mut first = Box::pin(
                client
                    .clone()
                    .get_item("shared".into(), SourceLibraryScope::default()),
            );
            let mut second = Box::pin(
                client
                    .clone()
                    .get_item("shared".into(), SourceLibraryScope::default()),
            );
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
            let mut old = Box::pin(
                client
                    .clone()
                    .get_item("shared".into(), SourceLibraryScope::default()),
            );
            assert!(futures::poll!(old.as_mut()).is_pending());
            client.cancel_all();
            assert_eq!(client.generation.load(Ordering::SeqCst), 1);
            let mut replacement = Box::pin(
                client
                    .clone()
                    .get_item("shared".into(), SourceLibraryScope::default()),
            );
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
