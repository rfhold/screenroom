//! Private gateway protocol. Callers persist intent before each effect and own serialization,
//! freshness fences, credential storage, fallback budgets, and cleanup supervision.
use super::{ApiKey, LiveJellyfin, network};
use reqwest::{Client, Method, Url, header};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

mod negotiation;
mod resources;
pub use negotiation::*;
pub use resources::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    NotDispatched,
    Rejected,
    Uncertain,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    InvalidInput,
    Identity,
    AuthorizationUnavailable,
    Unauthorized,
    Unsupported,
    ResourceRejected,
    Bounds,
    Deadline,
    Transport,
    Http(u16),
    InvalidResponse,
    FenceDenied,
    FenceUnavailable,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackError {
    pub failure: Failure,
    pub effect: Effect,
    /// Earlier mutation in the same method may already have taken effect.
    pub prior_effect: Option<Effect>,
}
impl std::fmt::Display for PlaybackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} ({:?}, prior: {:?})",
            self.failure, self.effect, self.prior_effect
        )
    }
}
impl std::error::Error for PlaybackError {}
pub type Result<T> = std::result::Result<T, PlaybackError>;
fn fail(failure: Failure) -> PlaybackError {
    PlaybackError {
        failure,
        effect: Effect::NotDispatched,
        prior_effect: None,
    }
}
fn id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Err(fail(Failure::InvalidInput))
    } else {
        Ok(())
    }
}
fn same_id(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    let compact = |value: &str| {
        if value.len() != 32
            && !(value.len() == 36
                && value.bytes().enumerate().all(|(index, b)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        b == b'-'
                    } else {
                        b.is_ascii_hexdigit()
                    }
                }))
        {
            return None;
        }
        let bytes: Vec<_> = value.bytes().filter(|b| *b != b'-').collect();
        (bytes.len() == 32 && bytes.iter().all(u8::is_ascii_hexdigit)).then(|| {
            bytes
                .into_iter()
                .map(|b| b.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
    };
    matches!((compact(left),compact(right)),(Some(a),Some(b)) if a==b)
}
fn remote_error(error: reqwest::Error, effect: bool) -> PlaybackError {
    let failure = if error.is_timeout() {
        Failure::Deadline
    } else {
        Failure::Transport
    };
    PlaybackError {
        failure,
        prior_effect: None,
        effect: if effect {
            Effect::Uncertain
        } else {
            Effect::NotDispatched
        },
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct GatewayDeviceId(String);
impl std::fmt::Debug for GatewayDeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GatewayDeviceId([REDACTED])")
    }
}
impl GatewayDeviceId {
    /// Generates 256 random bits without any provider IO. Persist before initiate.
    pub fn generate() -> Result<Self> {
        use std::io::Read;
        let mut bytes = [0u8; 32];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut bytes))
            .map_err(|_| fail(Failure::Transport))?;
        Self::from_recorded(format!(
            "screenroom-playback-{}",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ))
    }
    /// Only restore identities from the gateway's durable cleanup record, never client input.
    pub fn from_recorded(value: String) -> Result<Self> {
        if value.strip_prefix("screenroom-playback-").is_none_or(|s| {
            s.len() != 64
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        }) {
            return Err(fail(Failure::InvalidInput));
        }
        Ok(Self(value))
    }
    pub fn as_private_str(&self) -> &str {
        &self.0
    }
}

/// No Display/Serialize; exposure is solely for the private credential store.
pub struct SessionToken(ApiKey);
impl std::fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionToken([REDACTED])")
    }
}
impl SessionToken {
    pub fn from_private_store(value: String) -> Result<Self> {
        ApiKey::new(value)
            .map(Self)
            .map_err(|_| fail(Failure::InvalidInput))
    }
    pub fn expose_for_private_store(&self) -> &str {
        self.0.value()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationIntent {
    device: GatewayDeviceId,
    server: String,
    personal_profile: String,
}
impl std::fmt::Debug for AuthorizationIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AuthorizationIntent([REDACTED])")
    }
}
impl AuthorizationIntent {
    /// This is a private repository assertion, not proof of personal binding eligibility.
    /// The gateway must resolve an enabled personal binding before construction.
    pub fn new(
        device: GatewayDeviceId,
        server: String,
        personal_profile: String,
        _indexing_profile: &str,
    ) -> Result<Self> {
        id(&server)?;
        id(&personal_profile)?;
        Ok(Self {
            device,
            server,
            personal_profile,
        })
    }
    pub fn device(&self) -> &GatewayDeviceId {
        &self.device
    }
    pub fn server_id(&self) -> &str {
        &self.server
    }
    pub fn profile_id(&self) -> &str {
        &self.personal_profile
    }
}
pub struct PendingQuickConnect {
    intent: AuthorizationIntent,
    secret: SessionToken,
    code: String,
}
impl std::fmt::Debug for PendingQuickConnect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PendingQuickConnect([REDACTED])")
    }
}
impl PendingQuickConnect {
    pub fn intent(&self) -> &AuthorizationIntent {
        &self.intent
    }
    pub fn secret_for_private_store(&self) -> &str {
        self.secret.expose_for_private_store()
    }
}
pub struct AuthorizedQuickConnect(PendingQuickConnect);
impl std::fmt::Debug for AuthorizedQuickConnect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AuthorizedQuickConnect([REDACTED])")
    }
}
#[derive(Debug)]
pub struct SessionAuth {
    intent: AuthorizationIntent,
    token: SessionToken,
}
impl SessionAuth {
    pub fn intent(&self) -> &AuthorizationIntent {
        &self.intent
    }
    pub fn token(&self) -> &SessionToken {
        &self.token
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchPurpose {
    Active,
    Cleanup,
}

/// Private context; a cleanup fence checks the durable device/session record, not profile liveness.
pub struct DispatchContext<'a> {
    pub intent: &'a AuthorizationIntent,
    pub purpose: DispatchPurpose,
    /// Explicit control mutation; media GET may allocate encodings too. Fence every request.
    pub mutation: bool,
    pub play_session_id: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchDenial {
    Revoked,
    Unavailable,
}

pub type DispatchCheck<'a> =
    Pin<Box<dyn Future<Output = std::result::Result<(), DispatchDenial>> + Send + 'a>>;

/// One session-bound guard per adapter. Checks hold no transaction across provider IO.
/// A successful check cannot retract a request once dispatched.
pub trait DispatchGuard: Send + Sync {
    fn check<'a>(&'a self, context: DispatchContext<'a>) -> DispatchCheck<'a>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackTimeouts {
    pub control: Duration,
    pub headers: Duration,
    pub idle: Duration,
}
impl Default for PlaybackTimeouts {
    fn default() -> Self {
        Self {
            control: Duration::from_secs(10),
            headers: Duration::from_secs(10),
            idle: Duration::from_secs(10),
        }
    }
}

#[derive(Clone)]
pub struct PlaybackAdapter {
    admin: LiveJellyfin,
    client: Client,
    control_timeout: Duration,
    header_timeout: Duration,
    idle_timeout: Duration,
    body_limit: usize,
    intent: AuthorizationIntent,
    guard: Arc<dyn DispatchGuard>,
}
impl std::fmt::Debug for PlaybackAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PlaybackAdapter([REDACTED])")
    }
}
impl LiveJellyfin {
    /// No provider IO. Bound to one durable intent; no default bypass guard.
    /// Each deadline is 1ms..=10s, independent of the unchanged metadata client's deadline.
    pub fn playback_adapter_fenced(
        &self,
        intent: AuthorizationIntent,
        timeouts: PlaybackTimeouts,
        guard: Arc<dyn DispatchGuard>,
    ) -> Result<PlaybackAdapter> {
        for timeout in [timeouts.control, timeouts.headers, timeouts.idle] {
            if !(Duration::from_millis(1)..=Duration::from_secs(10)).contains(&timeout) {
                return Err(fail(Failure::InvalidInput));
            }
        }
        let timeout = timeouts.control.min(timeouts.headers).as_millis() as u64;
        let client = Client::builder()
            .no_proxy()
            .retry(reqwest::retry::never())
            .dns_resolver(std::sync::Arc::new(network::GuardedResolver::new(timeout)))
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_millis(timeout))
            .build()
            .map_err(|_| fail(Failure::InvalidInput))?;
        Ok(PlaybackAdapter {
            admin: self.clone(),
            client,
            control_timeout: timeouts.control,
            header_timeout: timeouts.headers,
            idle_timeout: timeouts.idle,
            body_limit: self.config.max_response_bytes.min(1024 * 1024),
            intent,
            guard,
        })
    }
}
impl PlaybackAdapter {
    fn check_binding(&self, intent: &AuthorizationIntent) -> Result<()> {
        if intent != &self.intent {
            return Err(fail(Failure::Identity));
        }
        Ok(())
    }
    async fn check_dispatch(
        &self,
        purpose: DispatchPurpose,
        mutation: bool,
        play_session_id: Option<&str>,
    ) -> Result<()> {
        tokio::time::timeout(
            self.control_timeout,
            self.guard.check(DispatchContext {
                intent: &self.intent,
                purpose,
                mutation,
                play_session_id,
            }),
        )
        .await
        .map_err(|_| fail(Failure::FenceUnavailable))?
        .map_err(|denial| {
            fail(match denial {
                DispatchDenial::Revoked => Failure::FenceDenied,
                DispatchDenial::Unavailable => Failure::FenceUnavailable,
            })
        })
    }
    fn url(&self, segments: &[&str]) -> Result<Url> {
        self.admin
            .url(segments)
            .map_err(|_| fail(Failure::InvalidInput))
    }
    fn authorization(
        &self,
        device: &GatewayDeviceId,
        token: Option<&SessionToken>,
    ) -> Result<header::HeaderValue> {
        let mut value = format!(
            "MediaBrowser Client=\"Screenroom\", Device=\"Playback Gateway\", DeviceId=\"{}\", Version=\"1\"",
            device.0
        );
        if let Some(token) = token {
            value.push_str(&format!(", Token=\"{}\"", token.0.value()));
        }
        let mut value =
            header::HeaderValue::from_str(&value).map_err(|_| fail(Failure::InvalidInput))?;
        value.set_sensitive(true);
        Ok(value)
    }
    async fn control(
        &self,
        method: Method,
        url: Url,
        auth: header::HeaderValue,
        body: Option<Value>,
        effect: bool,
        purpose: DispatchPurpose,
    ) -> Result<Vec<u8>> {
        let play_session = url
            .query_pairs()
            .find(|(key, _)| key.eq_ignore_ascii_case("playSessionId"))
            .map(|(_, value)| value.into_owned())
            .or_else(|| {
                body.as_ref()?
                    .get("PlaySessionId")?
                    .as_str()
                    .map(str::to_owned)
            });
        use tracing::Instrument as _;
        let span = tracing::info_span!(
            "provider playback", otel.kind = "client", otel.name = %method,
            http.request.method = %method, screenroom.dependency = "jellyfin"
        );
        let mut request = self
            .client
            .request(method, url)
            .header(header::AUTHORIZATION, auth)
            .header(header::ACCEPT, "application/json");
        if let Some(body) = body {
            request = request.json(&body);
        }
        self.check_dispatch(purpose, effect, play_session.as_deref())
            .await?;
        let run = async {
            let mut response = request
                .send()
                .instrument(span)
                .await
                .map_err(|e| remote_error(e, effect))?;
            let status = response.status().as_u16();
            if !(200..300).contains(&status) {
                return Err(PlaybackError {
                    prior_effect: None,
                    failure: if status == 401 || status == 403 {
                        Failure::Unauthorized
                    } else {
                        Failure::Http(status)
                    },
                    effect: if !effect {
                        Effect::NotDispatched
                    } else if (400..500).contains(&status) {
                        Effect::Rejected
                    } else {
                        Effect::Uncertain
                    },
                });
            }
            let error = |failure| PlaybackError {
                failure,
                prior_effect: None,
                effect: if effect {
                    Effect::Uncertain
                } else {
                    Effect::NotDispatched
                },
            };
            if response
                .content_length()
                .is_some_and(|n| n > self.body_limit as u64)
            {
                return Err(error(Failure::Bounds));
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|e| remote_error(e, effect))?
            {
                if chunk.len() > self.body_limit - bytes.len() {
                    return Err(error(Failure::Bounds));
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(bytes)
        };
        tokio::time::timeout(self.control_timeout, run)
            .await
            .unwrap_or({
                Err(PlaybackError {
                    prior_effect: None,
                    failure: Failure::Deadline,
                    effect: if effect {
                        Effect::Uncertain
                    } else {
                        Effect::NotDispatched
                    },
                })
            })
    }
    fn decode<T: DeserializeOwned>(&self, body: &[u8], effect: bool) -> Result<T> {
        serde_json::from_slice(body).map_err(|_| PlaybackError {
            prior_effect: None,
            failure: Failure::InvalidResponse,
            effect: if effect {
                Effect::Uncertain
            } else {
                Effect::NotDispatched
            },
        })
    }
    async fn verify_server(
        &self,
        intent: &AuthorizationIntent,
        purpose: DispatchPurpose,
    ) -> Result<()> {
        self.check_binding(intent)?;
        let body = self
            .control(
                Method::GET,
                self.url(&["System", "Info"])?,
                self.admin.authorization.clone(),
                None,
                false,
                purpose,
            )
            .await?;
        let server: Value = self.decode(&body, false)?;
        let name = server["ServerName"]
            .as_str()
            .map(|name| name.chars().filter(|c| !c.is_control()).collect::<String>());
        if server["Id"].as_str() != Some(&intent.server)
            || server["Version"]
                .as_str()
                .and_then(super::ServerVersion::parse)
                .is_none()
            || name.is_none_or(|name| name.trim().is_empty() || name.len() > 256)
        {
            return Err(fail(Failure::Identity));
        }
        Ok(())
    }
    async fn verify_profile(&self, intent: &AuthorizationIntent) -> Result<()> {
        self.check_binding(intent)?;
        let body = self
            .control(
                Method::GET,
                self.url(&["Users"])?,
                self.admin.authorization.clone(),
                None,
                false,
                DispatchPurpose::Active,
            )
            .await?;
        let profiles: Vec<super::ProfileDto> = self.decode(&body, false)?;
        if profiles
            .iter()
            .any(|profile| !profile.policy.is_disabled && super::valid_id(&profile.id).is_err())
            || !profiles
                .iter()
                .any(|profile| profile.id == intent.personal_profile && !profile.policy.is_disabled)
        {
            return Err(fail(Failure::Identity));
        }
        Ok(())
    }
    /// Persist device/cleanup intent BEFORE calling. No automatic enabling or admin-token fallback.
    pub async fn initiate(&self, intent: &AuthorizationIntent) -> Result<PendingQuickConnect> {
        self.verify_server(intent, DispatchPurpose::Active).await?;
        self.verify_profile(intent).await?;
        let body = self
            .control(
                Method::GET,
                self.url(&["QuickConnect", "Enabled"])?,
                self.authorization(&intent.device, None)?,
                None,
                false,
                DispatchPurpose::Active,
            )
            .await?;
        if !self.decode::<bool>(&body, false)? {
            return Err(fail(Failure::AuthorizationUnavailable));
        }
        let body = self
            .control(
                Method::POST,
                self.url(&["QuickConnect", "Initiate"])?,
                self.authorization(&intent.device, None)?,
                None,
                true,
                DispatchPurpose::Active,
            )
            .await
            .map_err(|mut e| {
                if e.failure == Failure::Unauthorized {
                    e.failure = Failure::AuthorizationUnavailable;
                }
                e
            })?;
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Pending {
            secret: String,
            code: String,
        }
        let pending: Pending = self.decode(&body, true)?;
        if pending.code.len() > 32
            || pending.code.is_empty()
            || !pending.code.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(PlaybackError {
                prior_effect: None,
                failure: Failure::InvalidResponse,
                effect: Effect::Uncertain,
            });
        }
        let secret =
            SessionToken::from_private_store(pending.secret).map_err(|_| PlaybackError {
                prior_effect: None,
                failure: Failure::InvalidResponse,
                effect: Effect::Uncertain,
            })?;
        Ok(PendingQuickConnect {
            intent: intent.clone(),
            secret,
            code: pending.code,
        })
    }
    /// Persist pending secret and authorization intent before this single attempt.
    pub async fn authorize(&self, pending: PendingQuickConnect) -> Result<AuthorizedQuickConnect> {
        self.verify_server(&pending.intent, DispatchPurpose::Active)
            .await?;
        self.verify_profile(&pending.intent).await?;
        let mut url = self.url(&["QuickConnect", "Authorize"])?;
        url.query_pairs_mut()
            .append_pair("code", &pending.code)
            .append_pair("userId", &pending.intent.personal_profile);
        let body = self
            .control(
                Method::POST,
                url,
                self.admin.authorization.clone(),
                None,
                true,
                DispatchPurpose::Active,
            )
            .await?;
        if !self.decode::<bool>(&body, true)? {
            return Err(PlaybackError {
                prior_effect: None,
                failure: Failure::AuthorizationUnavailable,
                effect: Effect::Rejected,
            });
        }
        Ok(AuthorizedQuickConnect(pending))
    }
    /// Persist authentication cleanup intent before this single attempt. Lost responses require
    /// exact-device recovery, not replaying the exchange or a playback-start report.
    pub async fn exchange(&self, authorized: AuthorizedQuickConnect) -> Result<SessionAuth> {
        let pending = authorized.0;
        self.check_binding(&pending.intent)?;
        let body = self
            .control(
                Method::POST,
                self.url(&["Users", "AuthenticateWithQuickConnect"])?,
                self.authorization(&pending.intent.device, None)?,
                Some(json!({"Secret":pending.secret.expose_for_private_store()})),
                true,
                DispatchPurpose::Active,
            )
            .await?;
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct User {
            id: String,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Authentication {
            access_token: String,
            server_id: String,
            user: User,
        }
        let reply: Authentication = self.decode(&body, true)?;
        if reply.server_id != pending.intent.server
            || reply.user.id != pending.intent.personal_profile
        {
            return Err(PlaybackError {
                prior_effect: None,
                failure: Failure::Identity,
                effect: Effect::Uncertain,
            });
        }
        let token =
            SessionToken::from_private_store(reply.access_token).map_err(|_| PlaybackError {
                prior_effect: None,
                failure: Failure::InvalidResponse,
                effect: Effect::Uncertain,
            })?;
        let auth = SessionAuth {
            intent: pending.intent,
            token,
        };
        self.verify_session(&auth).await.map_err(|mut e| {
            e.prior_effect = Some(Effect::Uncertain);
            e
        })?;
        Ok(auth)
    }
    /// Used only with durable private credentials. Read-only verification before restoration.
    pub async fn restore_session(
        &self,
        intent: AuthorizationIntent,
        token: SessionToken,
    ) -> Result<SessionAuth> {
        let auth = SessionAuth { intent, token };
        self.verify_session(&auth).await?;
        Ok(auth)
    }
    pub async fn verify_session(&self, auth: &SessionAuth) -> Result<()> {
        self.verify_session_for(auth, DispatchPurpose::Active).await
    }
    /// Durable credentials may still clean up a disabled profile's exact session.
    /// Active playback always requires a separate active dispatch fence.
    pub async fn restore_session_for_cleanup(
        &self,
        intent: AuthorizationIntent,
        token: SessionToken,
    ) -> Result<SessionAuth> {
        let auth = SessionAuth { intent, token };
        self.verify_session_for(&auth, DispatchPurpose::Cleanup)
            .await?;
        Ok(auth)
    }
    async fn verify_session_for(&self, auth: &SessionAuth, purpose: DispatchPurpose) -> Result<()> {
        self.verify_server(&auth.intent, purpose).await?;
        let body = self
            .control(
                Method::GET,
                self.url(&["Users", "Me"])?,
                self.authorization(&auth.intent.device, Some(&auth.token))?,
                None,
                false,
                purpose,
            )
            .await?;
        let user: Value = self.decode(&body, false)?;
        if user["Id"].as_str() != Some(&auth.intent.personal_profile)
            || (purpose == DispatchPurpose::Active
                && user["Policy"]["IsDisabled"].as_bool() != Some(false))
        {
            return Err(fail(Failure::Identity));
        }
        Ok(())
    }
    async fn user_control(
        &self,
        auth: &SessionAuth,
        method: Method,
        segments: &[&str],
        query: &[(&str, &str)],
        body: Option<Value>,
        effect: bool,
    ) -> Result<Vec<u8>> {
        self.check_binding(&auth.intent)?;
        let mut url = self.url(segments)?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().copied());
        }
        let purpose = if matches!(
            segments,
            ["Videos", "ActiveEncodings"] | ["Sessions", "Logout"]
        ) {
            DispatchPurpose::Cleanup
        } else {
            DispatchPurpose::Active
        };
        self.control(
            method,
            url,
            self.authorization(&auth.intent.device, Some(&auth.token))?,
            body,
            effect,
            purpose,
        )
        .await
    }
    /// Live personal resume, never a catalog percentage.
    pub async fn resume_position(&self, auth: &SessionAuth, item: &str) -> Result<Milliseconds> {
        id(item)?;
        let body = self
            .user_control(
                auth,
                Method::GET,
                &["Users", &auth.intent.personal_profile, "Items", item],
                &[],
                None,
                false,
            )
            .await?;
        let item_id = item;
        let item: Value = self.decode(&body, false)?;
        if item["Id"].as_str() != Some(item_id)
            || !matches!(item["Type"].as_str(), Some("Movie" | "Episode"))
            || item["PartCount"].as_u64().is_some_and(|n| n > 1)
        {
            return Err(fail(Failure::Unsupported));
        }
        let ticks = item["UserData"]["PlaybackPositionTicks"]
            .as_i64()
            .ok_or_else(|| fail(Failure::InvalidResponse))?;
        if item["RunTimeTicks"]
            .as_i64()
            .is_some_and(|runtime| runtime < ticks || runtime < 0)
        {
            return Err(fail(Failure::InvalidResponse));
        }
        Milliseconds::from_ticks(ticks)
    }
    pub async fn report(
        &self,
        auth: &SessionAuth,
        transport: &NegotiatedPlayback,
        report: PlaybackReport,
    ) -> Result<()> {
        transport.check_auth(auth)?;
        if !report.volume.is_finite() || !(0.0..=1.0).contains(&report.volume) {
            return Err(fail(Failure::InvalidInput));
        }
        let path = match report.kind {
            ReportKind::Start => vec!["Sessions", "Playing"],
            ReportKind::Progress => vec!["Sessions", "Playing", "Progress"],
            ReportKind::Stopped => vec!["Sessions", "Playing", "Stopped"],
        };
        let body = json!({
            "ItemId":transport.scope.item, "MediaSourceId":transport.scope.source,
            "PlaySessionId":transport.scope.play_session, "PositionTicks":report.position.ticks(),
            "PlayMethod":transport.method.provider_name(), "CanSeek":true,
            "AudioStreamIndex":transport.audio_index, "SubtitleStreamIndex":transport.subtitle_index,
            "IsPaused":report.paused,"IsMuted":report.muted,"VolumeLevel":(report.volume*100.0).round() as u32
        });
        self.user_control(auth, Method::POST, &path, &[], Some(body), true)
            .await?;
        Ok(())
    }
    pub async fn ping_encoding(
        &self,
        auth: &SessionAuth,
        transport: &NegotiatedPlayback,
    ) -> Result<()> {
        transport.check_auth(auth)?;
        if transport.method == PlaybackMethod::DirectPlay {
            return Ok(());
        }
        self.user_control(
            auth,
            Method::POST,
            &["Sessions", "Playing", "Ping"],
            &[("playSessionId", &transport.scope.play_session)],
            None,
            true,
        )
        .await?;
        Ok(())
    }
    pub async fn stop_encoding(
        &self,
        auth: &SessionAuth,
        transport: &NegotiatedPlayback,
    ) -> Result<()> {
        transport.check_auth(auth)?;
        self.stop_recorded_encoding(auth, &transport.encoding_identity())
            .await
    }
    /// Recovery path for an exact play-session identity from the private durable cleanup record.
    pub async fn stop_recorded_encoding(
        &self,
        auth: &SessionAuth,
        encoding: &EncodingIdentity,
    ) -> Result<()> {
        if encoding.device != auth.intent.device {
            return Err(fail(Failure::Identity));
        }
        self.verify_server(&auth.intent, DispatchPurpose::Cleanup)
            .await?;
        self.user_control(
            auth,
            Method::DELETE,
            &["Videos", "ActiveEncodings"],
            &[
                ("deviceId", &auth.intent.device.0),
                ("playSessionId", &encoding.play_session),
            ],
            None,
            true,
        )
        .await?;
        Ok(())
    }
    pub async fn logout(&self, auth: &SessionAuth) -> Result<()> {
        self.user_control(auth, Method::POST, &["Sessions", "Logout"], &[], None, true)
            .await?;
        Ok(())
    }
    /// Caller supplies only a durable gateway-owned identity. This never enumerates devices,
    /// uses profile-wide filters, or treats revocation as proof a personal-state write finished.
    pub async fn recover_device(&self, intent: &AuthorizationIntent) -> Result<()> {
        self.verify_server(intent, DispatchPurpose::Cleanup).await?;
        let mut url = self.url(&["Devices"])?;
        url.query_pairs_mut().append_pair("id", &intent.device.0);
        self.control(
            Method::DELETE,
            url,
            self.admin.authorization.clone(),
            None,
            true,
            DispatchPurpose::Cleanup,
        )
        .await?;
        Ok(())
    }
    /// Lost-token cleanup only. Never an administrative playback/reporting fallback.
    pub async fn recover_encoding(
        &self,
        intent: &AuthorizationIntent,
        encoding: &EncodingIdentity,
    ) -> Result<()> {
        if intent.device != encoding.device {
            return Err(fail(Failure::Identity));
        }
        self.verify_server(intent, DispatchPurpose::Cleanup).await?;
        let mut url = self.url(&["Videos", "ActiveEncodings"])?;
        url.query_pairs_mut()
            .append_pair("deviceId", &intent.device.0)
            .append_pair("playSessionId", &encoding.play_session);
        self.control(
            Method::DELETE,
            url,
            self.admin.authorization.clone(),
            None,
            true,
            DispatchPurpose::Cleanup,
        )
        .await?;
        Ok(())
    }
}
#[derive(Debug, Clone, Copy)]
pub enum ReportKind {
    Start,
    Progress,
    Stopped,
}
#[derive(Debug, Clone, Copy)]
pub struct PlaybackReport {
    pub kind: ReportKind,
    pub position: Milliseconds,
    pub paused: bool,
    pub muted: bool,
    pub volume: f64,
}

#[derive(Clone)]
pub struct EncodingIdentity {
    device: GatewayDeviceId,
    play_session: String,
}
impl std::fmt::Debug for EncodingIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EncodingIdentity([REDACTED])")
    }
}
impl EncodingIdentity {
    pub fn from_recorded(device: GatewayDeviceId, play_session: String) -> Result<Self> {
        id(&play_session)?;
        Ok(Self {
            device,
            play_session,
        })
    }
    pub fn device(&self) -> &GatewayDeviceId {
        &self.device
    }
    pub fn private_play_session_id(&self) -> &str {
        &self.play_session
    }
}

#[cfg(test)]
mod tests;
