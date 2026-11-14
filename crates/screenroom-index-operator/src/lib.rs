use axum::{
    Router,
    body::to_bytes,
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use screenroom_backend::{
    Error,
    ingest::IngestRepository,
    repair::{self as m, RepairRepository},
    sqlx::{self, PgPool},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use uuid::Uuid;

const BODY_LIMIT: usize = 65_536;

#[derive(Clone)]
pub struct Origin {
    value: String,
    authority: String,
}
impl Origin {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        let uri: Uri = value.parse().map_err(|_| "invalid operator origin")?;
        let authority = uri.authority().ok_or("invalid operator origin")?;
        let host = uri.host().unwrap_or("").trim_matches(['[', ']']);
        let loopback = host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        let base = if host.contains(':') {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        let valid_authority = authority.as_str() == base
            || uri
                .port_u16()
                .is_some_and(|port| port != 0 && authority.as_str() == format!("{base}:{port}"));
        if uri.scheme_str() != Some("http")
            || !loopback
            || !valid_authority
            || authority.as_str().contains('@')
            || value != format!("http://{authority}")
            || uri.port_u16() == Some(4173)
        {
            return Err("invalid operator origin");
        }
        Ok(Self {
            value: value.into(),
            authority: authority.to_string(),
        })
    }
    fn permits(&self, headers: &HeaderMap, mutation: bool) -> bool {
        fn one<'a>(headers: &'a HeaderMap, key: &str) -> Option<&'a str> {
            let mut values = headers.get_all(key).iter();
            let value = values.next()?.to_str().ok()?;
            if values.next().is_some() {
                return None;
            }
            Some(value)
        }
        (!headers.contains_key("sec-fetch-site")
            || matches!(one(headers, "sec-fetch-site"), Some("same-origin" | "none")))
            && one(headers, "host") == Some(self.authority.as_str())
            && if headers.contains_key("origin") || mutation {
                one(headers, "origin") == Some(self.value.as_str())
            } else {
                true
            }
    }
}

#[derive(Clone)]
pub struct Operator {
    pool: PgPool,
    origin: Origin,
    connections: screenroom_backend::connections::ConnectionRepository,
}
impl Operator {
    pub fn new(pool: PgPool, origin: Origin) -> Self {
        Self {
            connections: screenroom_backend::connections::ConnectionRepository::new(
                pool.clone(),
                None,
            ),
            pool,
            origin,
        }
    }
    pub fn with_connections(
        mut self,
        connections: screenroom_backend::connections::ConnectionRepository,
    ) -> Self {
        self.connections = connections;
        self
    }
    pub fn router(self) -> Router {
        Router::new().fallback(dispatch).with_state(self)
    }
    fn repository(&self) -> RepairRepository {
        RepairRepository::new(self.pool.clone())
    }
    async fn policy_owner(&self, source: Uuid, policy: Uuid) -> Result<(), Error> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT FROM ingest.policies WHERE id=$1 AND source_id=$2 AND selection_json->>'indexing'='1')",
        )
        .bind(policy)
        .bind(source)
        .fetch_one(&self.pool)
        .await?;
        if exists { Ok(()) } else { Err(Error::NotFound) }
    }
    async fn run(&self, source: Uuid, run: Uuid) -> Result<m::RunStatus, Error> {
        let status = self.repository().run_status(run).await?;
        if status.run.source_id != source {
            return Err(Error::NotFound);
        }
        Ok(status)
    }
}

fn error(status: StatusCode, code: &str) -> Response {
    (status, axum::Json(json!({"error":{"code":code}}))).into_response()
}
fn backend_error(e: Error) -> Response {
    match e {
        Error::Invalid(reason) => (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({"error":{"code":"invalid","reason":reason}})),
        )
            .into_response(),
        Error::Stale => error(StatusCode::CONFLICT, "stale"),
        Error::Connection(code) => error(
            match code {
                "request_conflict"
                | "request_expired"
                | "server_identity_mismatch"
                | "server_already_connected" => StatusCode::CONFLICT,
                "invalid_connection"
                | "invalid_discovery"
                | "invalid_credentials"
                | "credentials_rejected"
                | "invalid_server_identity"
                | "invalid_selection"
                | "profile_unavailable"
                | "library_unavailable"
                | "unsupported_library"
                | "unsupported_library_scope"
                | "selection_capacity"
                | "local_user_unavailable" => StatusCode::BAD_REQUEST,
                _ => StatusCode::SERVICE_UNAVAILABLE,
            },
            code,
        ),
        Error::NotFound => error(StatusCode::NOT_FOUND, "not_found"),
        Error::SourceBusy { run_id, .. } => (
            StatusCode::CONFLICT,
            axum::Json(json!({"error":{"code":"source_busy","run_id":run_id}})),
        )
            .into_response(),
        _ => error(StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
    }
}
fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(bytes).map_err(|_| Error::Invalid("json"))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalPolicy {
    pub id: Uuid,
    pub source_id: Uuid,
    pub version: i64,
    pub enabled: bool,
    pub automatic_runs: bool,
    pub next_due_at: screenroom_backend::chrono::DateTime<screenroom_backend::chrono::Utc>,
    pub configuration: Value,
}
pub type CanonicalRun = m::RunSummary;
fn canonical_policy(policy: screenroom_backend::ingest::Policy) -> Result<Value, Error> {
    let config = screenroom_backend::indexing::Configuration::from_value(&policy.selection_json)?;
    let configuration = json!({"indexing":config.indexing,"enabled":config.enabled,"automatic_runs":config.automatic_runs,"cadence_seconds":config.cadence_seconds,"reconcile_seconds":config.reconcile_seconds,"overlap_seconds":config.overlap_seconds,"source_concurrency":config.source_concurrency,"mode":config.mode,"components":config.components,"strategy":config.strategy,"stale_after_seconds":config.stale_after_seconds,"scope":config.scope,"limits":config.limits,"artwork":config.artwork});
    value(CanonicalPolicy {
        id: policy.id,
        source_id: policy.source_id,
        version: policy.version,
        enabled: policy.enabled,
        automatic_runs: policy.automatic_runs,
        next_due_at: policy.next_due_at,
        configuration,
    })
}
fn value(v: impl Serialize) -> Result<Value, Error> {
    serde_json::to_value(v).map_err(|_| Error::Unavailable)
}
fn id(s: &str) -> Result<Uuid, Error> {
    Uuid::parse_str(s).map_err(|_| Error::Invalid("id"))
}
fn default_limit() -> u32 {
    100
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    after: Option<String>,
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    pending_only: bool,
}
impl Page {
    fn uuid(&self) -> Result<Option<Uuid>, Error> {
        self.after.as_deref().map(id).transpose()
    }
}
fn page<T: Serialize>(items: Vec<T>, cursor: Option<String>, limit: u32) -> Value {
    let next = if items.len() == limit as usize {
        cursor
    } else {
        None
    };
    json!({"items":items,"next_after":next})
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Redrive {
    request_id: Uuid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Diagnostics {
    run_id: Option<Uuid>,
    capabilities: Option<Vec<String>>,
}

async fn dispatch(State(op): State<Operator>, request: Request) -> Response {
    let mutation = request.method() != Method::GET;
    if !op.origin.permits(request.headers(), mutation) {
        return error(StatusCode::FORBIDDEN, "invalid");
    }
    let path = request.uri().path().to_owned();
    if path.contains('%') || path.contains('\\') || path.split('/').any(|s| s == "." || s == "..") {
        return error(StatusCode::BAD_REQUEST, "invalid");
    }
    let mut response = if path == "/api" || path.starts_with("/api/") {
        if request.method() != Method::GET && request.method() != Method::POST {
            return error(StatusCode::METHOD_NOT_ALLOWED, "invalid");
        }
        if mutation
            && (request.headers().get_all("content-type").iter().count() != 1
                || request
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    != Some("application/json"))
        {
            return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "invalid");
        }
        let method = request.method().clone();
        let query = request.uri().query().unwrap_or("").to_owned();
        let body = match to_bytes(request.into_body(), BODY_LIMIT).await {
            Ok(b) => b,
            Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "invalid"),
        };
        match api(&op, &method, &path, &query, &body).await {
            Ok(v) => axum::Json(v).into_response(),
            Err(e) => backend_error(e),
        }
    } else {
        error(StatusCode::NOT_FOUND, "not_found")
    };
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response.headers_mut().insert("content-security-policy", "default-src 'self'; img-src 'self' data:; object-src 'none'; frame-ancestors 'none'; base-uri 'none'".parse().unwrap());
    response
}

async fn api(
    op: &Operator,
    method: &Method,
    path: &str,
    query: &str,
    body: &[u8],
) -> Result<Value, Error> {
    let p: Page = serde_urlencoded::from_str(query).map_err(|_| Error::Invalid("query"))?;
    if p.limit == 0 || p.limit > 1000 || p.after.as_ref().is_some_and(|a| a.len() > 200) {
        return Err(Error::Invalid("page"));
    }
    let repo = op.repository();
    let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
    if query
        .split('&')
        .any(|field| field.starts_with("pending_only="))
        && !path.ends_with("/candidates")
    {
        return Err(Error::Invalid("query"));
    }
    if !query.is_empty()
        && (path == "/api/health"
            || matches!(
                parts.as_slice(),
                ["api", "indexing", "sources", _, "runs", _]
            ))
    {
        return Err(Error::Invalid("query"));
    }
    if method == Method::GET {
        if !body.is_empty() {
            return Err(Error::Invalid("body"));
        }
        match parts.as_slice() {
            ["api", "connections", "jellyfin"] => {
                let rows = op.connections.list(p.uuid()?, p.limit).await?;
                let cursor = rows.last().map(|r| r.id.to_string());
                return Ok(page(rows, cursor, p.limit));
            }
            [
                "api",
                "indexing",
                "sources",
                source,
                "policies",
                policy,
                "progress",
            ] => {
                if !query.is_empty() {
                    return Err(Error::Invalid("query"));
                }
                op.policy_owner(id(source)?, id(policy)?).await?;
                return Ok(json!(
                    screenroom_backend::indexing::IndexingRepository::new(op.pool.clone())
                        .progress(id(policy)?)
                        .await?
                ));
            }
            [
                "api",
                "indexing",
                "sources",
                source,
                "runs",
                run,
                "execution",
            ] => {
                if !query.is_empty() {
                    return Err(Error::Invalid("query"));
                }
                op.run(id(source)?, id(run)?).await?;
                return Ok(json!(
                    screenroom_backend::indexing::IndexingRepository::new(op.pool.clone())
                        .execution(id(run)?)
                        .await?
                ));
            }
            ["api", "indexing", "sources", source, "policies"] => {
                let rows = screenroom_backend::indexing::IndexingRepository::new(op.pool.clone())
                    .policies(id(source)?, p.uuid()?, p.limit)
                    .await?;
                let cursor = rows.last().map(|r| r.id.to_string());
                let rows = rows
                    .into_iter()
                    .map(canonical_policy)
                    .collect::<Result<Vec<_>, Error>>()?;
                return Ok(page(rows, cursor, p.limit));
            }
            ["api", "indexing", "sources", source, "runs", run, "jobs"] => {
                let run = id(run)?;
                op.run(id(source)?, run).await?;
                let rows = IngestRepository::new(op.pool.clone())
                    .jobs(run, p.uuid()?, p.limit)
                    .await?;
                let cursor = rows.last().map(|r| r.id.to_string());
                let ingest = IngestRepository::new(op.pool.clone());
                let mut summaries = Vec::with_capacity(rows.len());
                for row in rows {
                    let eligibility = if row.state == "failed" {
                        ingest.redrive_eligibility(row.id).await?
                    } else {
                        screenroom_backend::RedriveEligibility {
                            allowed: false,
                            reason: Some("not_terminal_failed".into()),
                        }
                    };
                    let mut item = value(row)?;
                    item["redrive_allowed"] = json!(eligibility.allowed);
                    item["redrive_reason"] = json!(eligibility.reason);
                    summaries.push(item);
                }
                return Ok(page(summaries, cursor, p.limit));
            }
            [
                "api",
                "indexing",
                "sources",
                source,
                "runs",
                run,
                "jobs",
                job,
                history,
            ] => {
                let run = id(run)?;
                op.run(id(source)?, run).await?;
                let job = id(job)?;
                let owned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM queue.jobs WHERE id=$1 AND scope_id=$2 AND queue_name='indexing')").bind(job).bind(run).fetch_one(&op.pool).await?;
                if !owned {
                    return Err(Error::NotFound);
                }
                let ingest = IngestRepository::new(op.pool.clone());
                if *history == "attempts" {
                    let after = p
                        .after
                        .as_deref()
                        .unwrap_or("0")
                        .parse()
                        .map_err(|_| Error::Invalid("fence"))?;
                    let rows = ingest.job_attempts(job, after, p.limit).await?;
                    let cursor = rows.last().map(|r| r.fence.to_string());
                    let rows:Vec<_>=rows.into_iter().map(|r|json!({"job_id":r.job_id,"fence":r.fence,"generation":r.generation,"started_at":r.started_at,"finished_at":r.finished_at,"outcome":r.outcome,"error_code":r.error_code})).collect();
                    return Ok(page(rows, cursor, p.limit));
                }
                if *history == "redrives" {
                    let after = p
                        .after
                        .as_deref()
                        .unwrap_or("0")
                        .parse()
                        .map_err(|_| Error::Invalid("generation"))?;
                    let rows = ingest.job_redrives(job, after, p.limit).await?;
                    let cursor = rows.last().map(|r| r.generation.to_string());
                    return Ok(page(rows, cursor, p.limit));
                }
                return Err(Error::NotFound);
            }
            ["api", "health"] => {
                sqlx::query("SELECT 1").execute(&op.pool).await?;
                return Ok(json!({"ready":true}));
            }
            ["api", "sources"] => {
                let rows = repo.sources(p.uuid()?, p.limit).await?;
                let cursor = rows.last().map(|r| r.id.to_string());
                return Ok(page(rows, cursor, p.limit));
            }
            ["api", "sources", source, collection] => {
                let source = id(source)?;
                return match *collection {
                    "libraries" => {
                        let rows = repo.libraries(source, p.after.as_deref(), p.limit).await?;
                        let cursor = rows.last().map(|r| r.public_id.clone());
                        Ok(page(rows, cursor, p.limit))
                    }
                    _ => Err(Error::NotFound),
                };
            }
            ["api", "indexing", "sources", source, "runs"] => {
                let rows = repo.runs(Some(id(source)?), p.uuid()?, p.limit).await?;
                let cursor = rows.last().map(|r| r.id.to_string());
                return Ok(page(rows, cursor, p.limit));
            }
            ["api", "indexing", "sources", source, "runs", run] => {
                if !query.is_empty() {
                    return Err(Error::Invalid("query"));
                }
                return value(op.run(id(source)?, id(run)?).await?);
            }
            [
                "api",
                "indexing",
                "sources",
                source,
                "runs",
                run,
                "candidates",
            ] => {
                let run = id(run)?;
                op.run(id(source)?, run).await?;
                let rows = repo
                    .candidates(run, p.uuid()?, p.limit, p.pending_only)
                    .await?;
                let cursor = rows.last().map(|r| r.source_item_id.to_string());
                let rows: Vec<_>=rows.into_iter().map(|r|json!({"source_item_id":r.source_item_id,"public_id":r.public_id,"kind":r.kind,"state":r.state,"code":safe_code(r.code.as_deref()),"selection_reason":r.selection_reason,"selected_at":r.selected_at,"updated_at":r.updated_at})).collect();
                return Ok(page(rows, cursor, p.limit));
            }
            _ => return Err(Error::NotFound),
        }
    }
    if !query.is_empty() {
        return Err(Error::Invalid("query"));
    }
    match parts.as_slice() {
        ["api", "connections", "jellyfin", "discover"] => {
            value(op.connections.discover(&parse(body)?).await?)
        }
        ["api", "connections", "jellyfin"] => value(op.connections.save(&parse(body)?).await?),
        ["api", "indexing", "queue"] => {
            let request: Diagnostics = parse(body)?;
            if request.capabilities.as_ref().is_some_and(|caps| {
                caps.len() > 100
                    || caps.iter().any(|c| {
                        c.len() > 300
                            || c.is_empty()
                            || !c
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"_:-.,/".contains(&b))
                    })
            }) {
                return Err(Error::Invalid("capabilities"));
            }
            let ingest = IngestRepository::new(op.pool.clone());
            let observation = ingest
                .queue_observation(
                    request.run_id,
                    request.capabilities.as_deref().unwrap_or(&[]),
                )
                .await?;
            let mut snapshot = value(observation.counts)?;
            if request.capabilities.is_none() {
                for key in ["claimable", "reclaimable", "blocked"] {
                    snapshot[key] = Value::Null;
                }
            }
            let blockers = ingest
                .queue_blockers(request.run_id, request.capabilities.as_deref())
                .await?;
            Ok(
                json!({"queue_observation":{"started_at":observation.started_at,"finished_at":observation.finished_at},"blockers_as_of":blockers.as_of,"atomic":false,"effective_capabilities":request.capabilities,"worker_liveness":"unknown","queue":snapshot,"blockers":blockers,"count_semantics":{"blocked":"due-ready or expired-leased jobs with domain or capability blockers; attempt-limit-only jobs excluded; overlaps exhausted when another blocker exists","exhausted":"ready or expired-leased jobs at attempt limit, including delayed ready jobs"}}),
            )
        }
        ["api", "indexing", "sources", source, "policies"] => {
            let mut input: Value = parse(body)?;
            let config = input
                .get_mut("configuration")
                .and_then(Value::as_object_mut)
                .ok_or(Error::Invalid("configuration"))?;
            if config.contains_key("operator_profile_id") {
                return Err(Error::Invalid("private profile selection is server-side"));
            }
            let profile = op.connections.indexing_profile(id(source)?).await?;
            config.insert("operator_profile_id".into(), json!(profile));
            let request: screenroom_backend::indexing::ConfigureRequest =
                serde_json::from_value(input).map_err(|_| Error::Invalid("configuration"))?;
            canonical_policy(
                screenroom_backend::indexing::IndexingRepository::new(op.pool.clone())
                    .configure(id(source)?, &request)
                    .await?,
            )
        }
        ["api", "indexing", "sources", source, "start"] => {
            let request: screenroom_backend::indexing::StartRequest = parse(body)?;
            op.policy_owner(id(source)?, request.policy_id).await?;
            let run = screenroom_backend::indexing::IndexingRepository::new(op.pool.clone())
                .start(&request)
                .await?;
            value(op.run(id(source)?, run.id).await?.run)
        }
        [
            "api",
            "indexing",
            "sources",
            source,
            "runs",
            run,
            "jobs",
            job,
            "redrive",
        ] => {
            let request: Redrive = parse(body)?;
            let run = id(run)?;
            op.run(id(source)?, run).await?;
            let job = id(job)?;
            let owned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM queue.jobs WHERE id=$1 AND scope_id=$2 AND queue_name='indexing')").bind(job).bind(run).fetch_one(&op.pool).await?;
            if !owned {
                return Err(Error::NotFound);
            }
            value(
                IngestRepository::new(op.pool.clone())
                    .redrive_job(job, request.request_id)
                    .await?,
            )
        }
        ["api", "indexing", "sources", source, "runs", run, action]
            if ["pause", "resume", "cancel"].contains(action) =>
        {
            let _: Empty = parse(body)?;
            let source = id(source)?;
            let run = id(run)?;
            let status = op.run(source, run).await?;
            if !status.allowed_actions.iter().any(|a| a == action) {
                return Err(Error::Stale);
            }
            let ingest = IngestRepository::new(op.pool.clone());
            match *action {
                "pause" => ingest.pause(run).await?,
                "resume" => ingest.resume(run).await?,
                _ => ingest.cancel_run(run).await?,
            }
            let status = op.run(source, run).await?;
            if *action == "cancel" && status.run.state != "cancelled" {
                return Err(Error::Stale);
            }
            value(status)
        }
        _ => Err(Error::NotFound),
    }
}
fn safe_code(code: Option<&str>) -> Option<&str> {
    code.filter(|c| {
        [
            "ready",
            "no_poster",
            "unavailable",
            "invalid",
            "unauthorized",
            "transport",
            "timeout",
        ]
        .contains(c)
    })
}
