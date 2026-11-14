use crate::operator_models::*;
#[cfg(not(target_arch = "wasm32"))]
use futures::StreamExt;
use futures::future::{AbortHandle, Abortable, Either, select};
use serde::{Serialize, de::DeserializeOwned};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;
use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex},
};

const BODY_LIMIT: usize = 65_536;
pub(crate) const RESPONSE_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, thiserror::Error)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Error))]
pub enum OperatorError {
    #[error("stale operator state")]
    Stale,
    #[error("source busy: {run_id}")]
    SourceBusy { run_id: String },
    #[error("invalid operator argument")]
    Invalid,
    #[error("operator record not found")]
    NotFound,
    #[error("operator access forbidden")]
    Forbidden,
    #[error("operator unavailable")]
    Unavailable { ambiguous: bool },
    #[error("operator request cancelled")]
    Cancelled { ambiguous: bool },
    #[error("operator deadline exceeded")]
    Deadline { ambiguous: bool },
    #[error("invalid operator response")]
    InvalidResponse { ambiguous: bool },
    #[error("connection operation rejected: {code}")]
    Connection {
        code: OperatorConnectionErrorCode,
        ambiguous: bool,
    },
}

#[derive(Default)]
struct Requests {
    generation: u64,
    next: u64,
    active: HashMap<u64, AbortHandle>,
}
struct RequestGuard<'a> {
    state: &'a Mutex<Requests>,
    id: u64,
}
impl Drop for RequestGuard<'_> {
    fn drop(&mut self) {
        self.state.lock().unwrap().active.remove(&self.id);
    }
}

#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Object))]
pub struct OperatorClient {
    config: OperatorClientConfig,
    #[cfg(not(target_arch = "wasm32"))]
    origin: String,
    #[cfg(not(target_arch = "wasm32"))]
    http: reqwest::Client,
    requests: Mutex<Requests>,
}

fn uuid(value: &str) -> Result<(), OperatorError> {
    if crate::valid_uuid(value) {
        Ok(())
    } else {
        Err(OperatorError::Invalid)
    }
}
fn scope(scope: &OperatorScope) -> Result<(), OperatorError> {
    fn ids(values: &[String], max: usize) -> bool {
        values.len() <= max
            && values.iter().all(|v| {
                !v.is_empty()
                    && v.len() <= 200
                    && v.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-:.".contains(&b))
            })
            && values.iter().collect::<BTreeSet<_>>().len() == values.len()
    }
    if scope.library_ids.is_empty()
        || !ids(&scope.library_ids, 100)
        || !ids(&scope.item_ids, 1000)
        || scope.item_kinds.is_empty()
        || scope.item_kinds.len() > 2
        || (scope.item_kinds.len() == 2 && scope.item_kinds[0] == scope.item_kinds[1])
    {
        return Err(OperatorError::Invalid);
    }
    Ok(())
}
fn policy(id: &str, version: i64, selected: Option<&OperatorScope>) -> Result<(), OperatorError> {
    uuid(id)?;
    if version < 1 {
        return Err(OperatorError::Invalid);
    }
    if let Some(selected) = selected {
        scope(selected)?;
    }
    Ok(())
}

fn response_id(id: &str, expected: Option<&str>, ambiguous: bool) -> Result<(), OperatorError> {
    if uuid(id).is_err() || expected.is_some_and(|value| !value.eq_ignore_ascii_case(id)) {
        Err(OperatorError::InvalidResponse { ambiguous })
    } else {
        Ok(())
    }
}

fn checked_status(
    status: OperatorRunStatus,
    source: &str,
    run: Option<&str>,
    ambiguous: bool,
) -> Result<OperatorRunStatus, OperatorError> {
    response_id(&status.run.id, run, ambiguous)?;
    response_id(&status.run.source_id, Some(source), ambiguous)?;
    response_id(&status.run.policy_id, None, ambiguous)?;
    if status.run.policy_version < 1
        || status.run.items < 0
        || status.run.requests < 0
        || status
            .allowed_actions
            .iter()
            .any(|action| !["pause", "resume", "cancel"].contains(&action.as_str()))
    {
        return Err(OperatorError::InvalidResponse { ambiguous });
    }
    Ok(status)
}

#[cfg_attr(not(target_arch = "wasm32"), uniffi::export(async_runtime = "tokio"))]
impl OperatorClient {
    #[cfg_attr(not(target_arch = "wasm32"), uniffi::constructor)]
    pub fn new(config: OperatorClientConfig) -> Result<Arc<Self>, OperatorError> {
        let url = url::Url::parse(&config.endpoint).map_err(|_| OperatorError::Invalid)?;
        let origin = url.origin().ascii_serialization();
        if !(1..=120_000).contains(&config.timeout_ms)
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port() == Some(0)
        {
            return Err(OperatorError::Invalid);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let loopback = url.host_str().is_some_and(|host| {
                host == "localhost"
                    || host
                        .trim_matches(['[', ']'])
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            });
            if url.scheme() != "http" || !loopback || config.endpoint != format!("{origin}/api") {
                return Err(OperatorError::Invalid);
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let location = js_sys::Reflect::get(&js_sys::global(), &"location".into())
                .map_err(|_| OperatorError::Invalid)?;
            let current = js_sys::Reflect::get(&location, &"origin".into())
                .ok()
                .and_then(|v| v.as_string())
                .ok_or(OperatorError::Invalid)?;
            if !["http", "https"].contains(&url.scheme())
                || origin != current
                || config.endpoint != format!("{origin}/__operator/api")
            {
                return Err(OperatorError::Invalid);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let http = reqwest::Client::builder()
            .no_proxy()
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| OperatorError::Invalid)?;
        Ok(Arc::new(Self {
            config,
            #[cfg(not(target_arch = "wasm32"))]
            origin,
            #[cfg(not(target_arch = "wasm32"))]
            http,
            requests: Mutex::new(Requests::default()),
        }))
    }
    pub fn configuration(&self) -> OperatorClientConfig {
        self.config.clone()
    }
    /// Stable endpoint namespace, not a backend instance identifier or authorization proof.
    pub fn configuration_namespace(&self) -> String {
        self.config.endpoint.clone()
    }
    pub fn cancel_all(&self) {
        let mut state = self.requests.lock().unwrap();
        state.generation = state.generation.wrapping_add(1);
        for (_, abort) in state.active.drain() {
            abort.abort();
        }
    }
    pub async fn health(&self) -> Result<OperatorHealth, OperatorError> {
        self.request("health", None, None, false).await
    }
    pub async fn discover_connection(
        &self,
        input: OperatorDiscoverConnectionRequest,
    ) -> Result<OperatorConnectionDiscovery, OperatorError> {
        connection_credentials(&input.base_url, &input.api_key)?;
        #[derive(Serialize)]
        struct Transport<'a> {
            base_url: &'a str,
            api_key: &'a str,
        }
        let result: OperatorConnectionDiscovery = self
            .request(
                "connections/jellyfin/discover",
                None,
                Some(body(&Transport {
                    base_url: &input.base_url,
                    api_key: &input.api_key,
                })?),
                false,
            )
            .await?;
        let exposed = [
            &result.server.id,
            &result.server.name,
            &result.server.version,
        ]
        .into_iter()
        .chain(result.libraries.iter().flat_map(|l| [&l.id, &l.name]))
        .chain(result.profiles.iter().flat_map(|p| [&p.id, &p.name]));
        if exposed.into_iter().any(|s| s.contains(&input.api_key)) {
            return Err(OperatorError::InvalidResponse { ambiguous: false });
        }
        Ok(result)
    }
    pub async fn save_connection(
        &self,
        input: OperatorSaveConnectionRequest,
    ) -> Result<OperatorSavedConnection, OperatorError> {
        uuid(&input.request_id)?;
        uuid(&input.local_user_id)?;
        connection_credentials(&input.base_url, &input.api_key)?;
        if input.library_ids.len() > 32 {
            return Err(connection_error(
                OperatorConnectionErrorCode::SelectionCapacity,
            ));
        }
        if !connection_id(&input.server_id)
            || !connection_id(&input.profile_id)
            || input.library_ids.is_empty()
            || input.library_ids.iter().any(|s| !connection_id(s))
            || input.library_ids.iter().collect::<BTreeSet<_>>().len() != input.library_ids.len()
        {
            return Err(connection_error(
                OperatorConnectionErrorCode::InvalidSelection,
            ));
        }
        #[derive(Serialize)]
        struct Transport<'a> {
            request_id: &'a str,
            base_url: &'a str,
            api_key: &'a str,
            server_id: &'a str,
            profile_id: &'a str,
            library_ids: &'a [String],
            local_user_id: &'a str,
            refresh: bool,
        }
        let result: OperatorSavedConnection = self
            .request(
                "connections/jellyfin",
                None,
                Some(body(&Transport {
                    request_id: &input.request_id,
                    base_url: &input.base_url,
                    api_key: &input.api_key,
                    server_id: &input.server_id,
                    profile_id: &input.profile_id,
                    library_ids: &input.library_ids,
                    local_user_id: &input.local_user_id,
                    refresh: input.refresh,
                })?),
                true,
            )
            .await?;
        checked_connection(&result.connection, true)?;
        response_id(&result.policy_id, None, true)?;
        if let Some(run) = &result.run_id {
            response_id(run, None, true)?;
        }
        if result.policy_version < 1
            || result.run_id.is_some() != input.refresh
            || result.connection.display_name.contains(&input.api_key)
            || result.connection.server_version.contains(&input.api_key)
        {
            return Err(OperatorError::InvalidResponse { ambiguous: true });
        }
        Ok(result)
    }
    pub async fn connections(
        &self,
        page: OperatorPage,
    ) -> Result<OperatorConnectionPage, OperatorError> {
        indexing_page(&page, false, false)?;
        let result: OperatorConnectionPage = self
            .request(
                "connections/jellyfin",
                Some((&page, true, None)),
                None,
                false,
            )
            .await?;
        self.check_page(
            result.items.len(),
            result.next_after.as_deref(),
            &page,
            true,
        )?;
        for item in &result.items {
            checked_connection(item, false)?;
        }
        Ok(result)
    }
    pub async fn sources(&self, page: OperatorPage) -> Result<OperatorSourcePage, OperatorError> {
        let result: OperatorSourcePage = self
            .request("sources", Some((&page, true, None)), None, false)
            .await?;
        self.check_page(
            result.items.len(),
            result.next_after.as_deref(),
            &page,
            true,
        )?;
        for item in &result.items {
            uuid(&item.id).map_err(|_| OperatorError::InvalidResponse { ambiguous: false })?;
        }
        Ok(result)
    }
    pub async fn libraries(
        &self,
        source_id: String,
        page: OperatorPage,
    ) -> Result<OperatorLibraryPage, OperatorError> {
        uuid(&source_id)?;
        let result: OperatorLibraryPage = self
            .request(
                &format!("sources/{source_id}/libraries"),
                Some((&page, false, None)),
                None,
                false,
            )
            .await?;
        self.check_page(
            result.items.len(),
            result.next_after.as_deref(),
            &page,
            false,
        )?;
        Ok(result)
    }
    pub async fn indexing_runs(
        &self,
        source_id: String,
        page: OperatorPage,
    ) -> Result<OperatorRunPage, OperatorError> {
        uuid(&source_id)?;
        let result: OperatorRunPage = self
            .request(
                &format!("indexing/sources/{source_id}/runs"),
                Some((&page, true, None)),
                None,
                false,
            )
            .await?;
        self.check_page(
            result.items.len(),
            result.next_after.as_deref(),
            &page,
            true,
        )?;
        for item in &result.items {
            response_id(&item.id, None, false)?;
            response_id(&item.source_id, Some(&source_id), false)?;
            response_id(&item.policy_id, None, false)?;
        }
        Ok(result)
    }
    pub async fn indexing_candidates(
        &self,
        source_id: String,
        run_id: String,
        page: OperatorPage,
        pending_only: bool,
    ) -> Result<OperatorCandidatePage, OperatorError> {
        uuid(&source_id)?;
        uuid(&run_id)?;
        let result: OperatorCandidatePage = self
            .request(
                &format!("indexing/sources/{source_id}/runs/{run_id}/candidates"),
                Some((&page, true, Some(pending_only))),
                None,
                false,
            )
            .await?;
        self.check_page(
            result.items.len(),
            result.next_after.as_deref(),
            &page,
            true,
        )?;
        for item in &result.items {
            response_id(&item.source_item_id, None, false)?;
        }
        Ok(result)
    }
    pub async fn indexing_policies(
        &self,
        source_id: String,
        page: OperatorPage,
    ) -> Result<OperatorIndexingPolicyPage, OperatorError> {
        uuid(&source_id)?;
        indexing_page(&page, false, false)?;
        let result: OperatorIndexingPolicyPage = self
            .request(
                &format!("indexing/sources/{source_id}/policies"),
                Some((&page, true, None)),
                None,
                false,
            )
            .await?;
        self.check_page(
            result.items.len(),
            result.next_after.as_deref(),
            &page,
            true,
        )?;
        for item in &result.items {
            checked_indexing_policy(item, &source_id, None, false)?;
        }
        Ok(result)
    }
    pub async fn indexing_configure(
        &self,
        source_id: String,
        input: OperatorIndexingConfigureRequest,
    ) -> Result<OperatorIndexingPolicy, OperatorError> {
        uuid(&source_id)?;
        uuid(&input.request_id)?;
        indexing_configuration(&input.configuration)?;
        if let Some(existing) = &input.existing {
            policy(&existing.policy_id, existing.expected_version, None)?;
        }
        let result = self
            .request(
                &format!("indexing/sources/{source_id}/policies"),
                None,
                Some(body(&input)?),
                true,
            )
            .await?;
        checked_indexing_policy(
            &result,
            &source_id,
            input.existing.as_ref().map(|e| e.policy_id.as_str()),
            true,
        )?;
        Ok(result)
    }
    pub async fn indexing_progress(
        &self,
        source_id: String,
        policy_id: String,
    ) -> Result<Option<OperatorIndexingProgress>, OperatorError> {
        uuid(&source_id)?;
        uuid(&policy_id)?;
        self.request(
            &format!("indexing/sources/{source_id}/policies/{policy_id}/progress"),
            None,
            None,
            false,
        )
        .await
    }
    pub async fn indexing_start(
        &self,
        source_id: String,
        input: OperatorStartRequest,
    ) -> Result<OperatorRun, OperatorError> {
        uuid(&source_id)?;
        uuid(&input.request_id)?;
        policy(&input.policy_id, input.policy_version, input.scope.as_ref())?;
        let result: OperatorRun = self
            .request(
                &format!("indexing/sources/{source_id}/start"),
                None,
                Some(body(&input)?),
                true,
            )
            .await?;
        response_id(&result.id, None, true)?;
        response_id(&result.source_id, Some(&source_id), true)?;
        response_id(&result.policy_id, Some(&input.policy_id), true)?;
        if result.policy_version != input.policy_version || result.items < 0 || result.requests < 0
        {
            return Err(OperatorError::InvalidResponse { ambiguous: true });
        }
        Ok(result)
    }
    pub async fn indexing_run_status(
        &self,
        source_id: String,
        run_id: String,
    ) -> Result<OperatorRunStatus, OperatorError> {
        let path = indexing_run_path(&source_id, &run_id)?;
        checked_status(
            self.request(&path, None, None, false).await?,
            &source_id,
            Some(&run_id),
            false,
        )
    }
    pub async fn indexing_execution(
        &self,
        source_id: String,
        run_id: String,
    ) -> Result<Option<OperatorIndexingExecution>, OperatorError> {
        let path = indexing_run_path(&source_id, &run_id)?;
        self.request(&format!("{path}/execution"), None, None, false)
            .await
    }
    pub async fn indexing_pause(
        &self,
        source_id: String,
        run_id: String,
    ) -> Result<OperatorRunStatus, OperatorError> {
        self.indexing_control(source_id, run_id, "pause").await
    }
    pub async fn indexing_resume(
        &self,
        source_id: String,
        run_id: String,
    ) -> Result<OperatorRunStatus, OperatorError> {
        self.indexing_control(source_id, run_id, "resume").await
    }
    pub async fn indexing_cancel(
        &self,
        source_id: String,
        run_id: String,
    ) -> Result<OperatorRunStatus, OperatorError> {
        self.indexing_control(source_id, run_id, "cancel").await
    }
    pub async fn indexing_jobs(
        &self,
        source_id: String,
        run_id: String,
        page: OperatorPage,
    ) -> Result<OperatorJobPage, OperatorError> {
        let path = indexing_run_path(&source_id, &run_id)?;
        indexing_page(&page, false, false)?;
        let result: OperatorJobPage = self
            .request(
                &format!("{path}/jobs"),
                Some((&page, true, None)),
                None,
                false,
            )
            .await?;
        self.check_page(
            result.items.len(),
            result.next_after.as_deref(),
            &page,
            true,
        )?;
        for job in &result.items {
            response_id(&job.id, None, false)?;
            response_id(
                job.run_id
                    .as_deref()
                    .ok_or(OperatorError::InvalidResponse { ambiguous: false })?,
                Some(&run_id),
                false,
            )?;
            if job.attempts < 0 || job.max_attempts < 1 || job.generation < 0 {
                return Err(OperatorError::InvalidResponse { ambiguous: false });
            }
        }
        Ok(result)
    }
    pub async fn indexing_attempts(
        &self,
        source_id: String,
        run_id: String,
        job_id: String,
        page: OperatorPage,
    ) -> Result<OperatorAttemptPage, OperatorError> {
        let path = indexing_job_path(&source_id, &run_id, &job_id)?;
        indexing_page(&page, true, false)?;
        let result: OperatorAttemptPage = self
            .request(
                &format!("{path}/attempts"),
                Some((&page, false, None)),
                None,
                false,
            )
            .await?;
        self.check_page(
            result.items.len(),
            result.next_after.as_deref(),
            &page,
            false,
        )?;
        checked_history(
            &page,
            result.next_after.as_deref(),
            result.items.iter().map(|a| a.fence),
        )?;
        for attempt in &result.items {
            response_id(&attempt.job_id, Some(&job_id), false)?;
        }
        Ok(result)
    }
    pub async fn indexing_redrives(
        &self,
        source_id: String,
        run_id: String,
        job_id: String,
        page: OperatorPage,
    ) -> Result<OperatorRedrivePage, OperatorError> {
        let path = indexing_job_path(&source_id, &run_id, &job_id)?;
        indexing_page(&page, true, true)?;
        let result: OperatorRedrivePage = self
            .request(
                &format!("{path}/redrives"),
                Some((&page, false, None)),
                None,
                false,
            )
            .await?;
        self.check_page(
            result.items.len(),
            result.next_after.as_deref(),
            &page,
            false,
        )?;
        checked_history(
            &page,
            result.next_after.as_deref(),
            result.items.iter().map(|r| i64::from(r.generation)),
        )?;
        for redrive in &result.items {
            response_id(&redrive.job_id, Some(&job_id), false)?;
            response_id(&redrive.request_id, None, false)?;
        }
        Ok(result)
    }
    pub async fn indexing_redrive(
        &self,
        source_id: String,
        run_id: String,
        job_id: String,
        input: OperatorRedriveRequest,
    ) -> Result<OperatorRedrive, OperatorError> {
        let path = indexing_job_path(&source_id, &run_id, &job_id)?;
        uuid(&input.request_id)?;
        let result: OperatorRedrive = self
            .request(&format!("{path}/redrive"), None, Some(body(&input)?), true)
            .await?;
        response_id(&result.job_id, Some(&job_id), true)?;
        response_id(&result.request_id, Some(&input.request_id), true)?;
        if result.generation < 1 {
            return Err(OperatorError::InvalidResponse { ambiguous: true });
        }
        Ok(result)
    }
    pub async fn indexing_queue(
        &self,
        input: OperatorDiagnosticsRequest,
    ) -> Result<OperatorQueueDiagnostics, OperatorError> {
        if let Some(run) = &input.run_id {
            uuid(run)?;
        }
        if input.capabilities.as_ref().is_some_and(|caps| {
            caps.len() > 100
                || caps.iter().any(|c| {
                    c.is_empty()
                        || c.len() > 300
                        || !c
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_:-.,/".contains(&b))
                })
        }) {
            return Err(OperatorError::Invalid);
        }
        let result: OperatorQueueDiagnostics = self
            .request("indexing/queue", None, Some(body(&input)?), false)
            .await?;
        let relative = [
            &result.queue.claimable,
            &result.queue.reclaimable,
            &result.queue.blocked,
            &result.blockers.capability,
        ];
        if result.atomic
            || result.effective_capabilities != input.capabilities
            || relative
                .iter()
                .any(|n| n.is_some() != input.capabilities.is_some())
        {
            return Err(OperatorError::InvalidResponse { ambiguous: false });
        }
        Ok(result)
    }
}

fn indexing_run_path(source: &str, run: &str) -> Result<String, OperatorError> {
    uuid(source)?;
    uuid(run)?;
    Ok(format!("indexing/sources/{source}/runs/{run}"))
}
fn checked_connection(item: &OperatorConnection, ambiguous: bool) -> Result<(), OperatorError> {
    response_id(&item.id, None, ambiguous)?;
    response_id(&item.source_id, None, ambiguous)
}
fn connection_error(code: OperatorConnectionErrorCode) -> OperatorError {
    OperatorError::Connection {
        code,
        ambiguous: false,
    }
}
fn connection_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn connection_credentials(base: &str, key: &str) -> Result<(), OperatorError> {
    if key.is_empty()
        || key.len() > 4096
        || key
            .bytes()
            .any(|b| !(32..127).contains(&b) || b == b'"' || b == b'\\')
    {
        return Err(connection_error(
            OperatorConnectionErrorCode::InvalidCredentials,
        ));
    }
    let url = url::Url::parse(base)
        .map_err(|_| connection_error(OperatorConnectionErrorCode::InvalidConnection))?;
    if base.is_empty()
        || base.len() > 2048
        || base.contains(key)
        || base.bytes().any(|b| b.is_ascii_control())
        || !["http", "https"].contains(&url.scheme())
        || url.host_str().is_none()
        || url.port() == Some(0)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(connection_error(
            OperatorConnectionErrorCode::InvalidConnection,
        ));
    }
    Ok(())
}
fn indexing_job_path(source: &str, run: &str, job: &str) -> Result<String, OperatorError> {
    uuid(job)?;
    Ok(format!("{}/jobs/{job}", indexing_run_path(source, run)?))
}
fn indexing_page(
    page: &OperatorPage,
    numeric: bool,
    generation: bool,
) -> Result<(), OperatorError> {
    if !(1..=200).contains(&page.limit) {
        return Err(OperatorError::Invalid);
    }
    if let Some(after) = &page.after {
        if numeric {
            if !decimal_cursor(
                after,
                if generation {
                    i64::from(i32::MAX)
                } else {
                    i64::MAX
                },
            ) {
                return Err(OperatorError::Invalid);
            }
        } else {
            uuid(after)?;
        }
    }
    Ok(())
}
fn decimal_cursor(value: &str, max: i64) -> bool {
    !value.is_empty()
        && value.len() <= 19
        && value.bytes().all(|b| b.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
        && value.parse::<i64>().is_ok_and(|n| n <= max)
}
fn checked_history(
    page: &OperatorPage,
    next: Option<&str>,
    items: impl Iterator<Item = i64>,
) -> Result<(), OperatorError> {
    let mut previous = page
        .after
        .as_deref()
        .unwrap_or("0")
        .parse::<i64>()
        .map_err(|_| OperatorError::Invalid)?;
    for item in items {
        if item <= previous {
            return Err(OperatorError::InvalidResponse { ambiguous: false });
        }
        previous = item;
    }
    if next.is_some_and(|s| s != previous.to_string()) {
        return Err(OperatorError::InvalidResponse { ambiguous: false });
    }
    Ok(())
}
fn checked_indexing_policy(
    item: &OperatorIndexingPolicy,
    source: &str,
    expected: Option<&str>,
    ambiguous: bool,
) -> Result<(), OperatorError> {
    response_id(&item.id, expected, ambiguous)?;
    response_id(&item.source_id, Some(source), ambiguous)?;
    if item.version < 1
        || indexing_configuration(&item.configuration).is_err()
        || item.enabled != item.configuration.enabled
        || item.automatic_runs != item.configuration.automatic_runs
    {
        return Err(OperatorError::InvalidResponse { ambiguous });
    }
    Ok(())
}
fn indexing_configuration(c: &OperatorIndexingConfiguration) -> Result<(), OperatorError> {
    scope(&c.scope)?;
    let l = &c.limits;
    let r = &l.retry;
    if c.indexing != 1
        || !(60..=2_592_000).contains(&c.cadence_seconds)
        || !(60..=2_592_000).contains(&c.reconcile_seconds)
        || !(1..=86400).contains(&c.overlap_seconds)
        || c.source_concurrency != 1
        || !(7..=1000).contains(&l.work_requests)
        || !(3..=1000).contains(&l.work_records)
        || !(1..=100_000_000).contains(&l.lifetime_requests)
        || !(1..=100_000_000).contains(&l.lifetime_records)
        || !(60..=2_592_000).contains(&l.deadline_seconds)
        || !(1..=240).contains(&l.work_timeout_seconds)
        || !(1..=20).contains(&r.max_attempts)
        || !(1..=300).contains(&r.base_seconds)
        || !(r.base_seconds..=86400).contains(&r.cap_seconds)
        || !(0..=100).contains(&r.jitter_percent)
    {
        return Err(OperatorError::Invalid);
    }
    if c.mode == OperatorIndexingMode::Repair {
        if c.components.metadata
            || !c.components.artwork
            || c.components.embeddings
            || c.artwork.posters.skip_existing != (c.strategy == OperatorIndexingStrategy::Missing)
        {
            return Err(OperatorError::Invalid);
        }
    } else if !c.components.metadata
        || c.strategy != OperatorIndexingStrategy::All
        || !c.scope.item_ids.is_empty()
        || c.scope.item_kinds.len() != 2
        || !c.artwork.posters.skip_existing
    {
        return Err(OperatorError::Invalid);
    }
    if c.strategy == OperatorIndexingStrategy::Stale {
        if c.mode != OperatorIndexingMode::Repair
            || !c
                .stale_after_seconds
                .is_some_and(|n| (60..=2_592_000).contains(&n))
        {
            return Err(OperatorError::Invalid);
        }
    } else if c.stale_after_seconds.is_some() {
        return Err(OperatorError::Invalid);
    }
    Ok(())
}

fn body(input: &impl Serialize) -> Result<Vec<u8>, OperatorError> {
    let bytes = serde_json::to_vec(input).map_err(|_| OperatorError::Invalid)?;
    if bytes.len() > BODY_LIMIT {
        Err(OperatorError::Invalid)
    } else {
        Ok(bytes)
    }
}

impl OperatorClient {
    async fn indexing_control(
        &self,
        source: String,
        run: String,
        action: &str,
    ) -> Result<OperatorRunStatus, OperatorError> {
        let path = indexing_run_path(&source, &run)?;
        checked_status(
            self.request(
                &format!("{path}/{action}"),
                None,
                Some(b"{}".to_vec()),
                true,
            )
            .await?,
            &source,
            Some(&run),
            true,
        )
    }
    fn check_page(
        &self,
        len: usize,
        after: Option<&str>,
        page: &OperatorPage,
        uuid_cursor: bool,
    ) -> Result<(), OperatorError> {
        if len > page.limit as usize
            || after.is_some_and(|a| {
                len != page.limit as usize
                    || a.is_empty()
                    || a.len() > 200
                    || (uuid_cursor && uuid(a).is_err())
                    || page.after.as_deref() == Some(a)
            })
        {
            return Err(OperatorError::InvalidResponse { ambiguous: false });
        }
        Ok(())
    }
    async fn request<T: DeserializeOwned>(
        &self,
        path: &str,
        page: Option<(&OperatorPage, bool, Option<bool>)>,
        body: Option<Vec<u8>>,
        ambiguous: bool,
    ) -> Result<T, OperatorError> {
        let mut url = url::Url::parse(&format!("{}/{path}", self.config.endpoint))
            .map_err(|_| OperatorError::Invalid)?;
        if let Some((page, uuid_cursor, pending)) = page {
            if !(1..=1000).contains(&page.limit)
                || page.after.as_ref().is_some_and(|a| {
                    a.is_empty() || a.len() > 200 || (uuid_cursor && uuid(a).is_err())
                })
            {
                return Err(OperatorError::Invalid);
            }
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", &page.limit.to_string());
            if let Some(after) = &page.after {
                query.append_pair("after", after);
            }
            if let Some(pending) = pending {
                query.append_pair("pending_only", if pending { "true" } else { "false" });
            }
        }
        let (abort, registration) = AbortHandle::new_pair();
        let (id, generation) = {
            let mut state = self.requests.lock().unwrap();
            if state.active.len() >= 64 {
                return Err(OperatorError::Unavailable { ambiguous: false });
            }
            let id = state.next;
            state.next = state.next.wrapping_add(1);
            state.active.insert(id, abort);
            (id, state.generation)
        };
        let _guard = RequestGuard {
            state: &self.requests,
            id,
        };
        let operation = async {
            #[cfg(not(target_arch = "wasm32"))]
            let (status, bytes) = {
                let mut request = if let Some(body) = body {
                    self.http
                        .post(url)
                        .header("content-type", "application/json")
                        .body(body)
                } else {
                    self.http.get(url)
                };
                request = request
                    .header("cache-control", "no-store")
                    .header("accept", "application/json");
                #[cfg(not(target_arch = "wasm32"))]
                {
                    request = request.header("origin", &self.origin);
                }
                let response = request
                    .send()
                    .await
                    .map_err(|_| OperatorError::Unavailable { ambiguous })?;
                let status = response.status();
                let invalid = || OperatorError::InvalidResponse { ambiguous };
                if response
                    .content_length()
                    .is_some_and(|n| n > RESPONSE_LIMIT as u64)
                {
                    return Err(invalid());
                }
                let mut stream = response.bytes_stream();
                let mut bytes = Vec::new();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(|_| OperatorError::Unavailable { ambiguous })?;
                    if chunk.len() > RESPONSE_LIMIT - bytes.len() {
                        return Err(invalid());
                    }
                    bytes.extend_from_slice(&chunk);
                }
                (status.as_u16(), bytes)
            };
            #[cfg(target_arch = "wasm32")]
            let (status, bytes) =
                crate::operator_wasm::fetch(url.as_str(), body, ambiguous).await?;
            if !(200..300).contains(&status) {
                return Err(response_error(status, &bytes, ambiguous));
            }
            serde_json::from_slice(&bytes).map_err(|_| OperatorError::InvalidResponse { ambiguous })
        };
        #[cfg(not(target_arch = "wasm32"))]
        let deadline = tokio::time::sleep(Duration::from_millis(u64::from(self.config.timeout_ms)));
        #[cfg(target_arch = "wasm32")]
        let deadline = gloo_timers::future::TimeoutFuture::new(self.config.timeout_ms);
        let operation = Abortable::new(operation, registration);
        futures::pin_mut!(operation, deadline);
        let result = match select(operation, deadline).await {
            Either::Left((Ok(result), _)) => result,
            Either::Left((Err(_), _)) => Err(OperatorError::Cancelled { ambiguous }),
            Either::Right(_) => Err(OperatorError::Deadline { ambiguous }),
        };
        if self.requests.lock().unwrap().generation != generation {
            return Err(OperatorError::Cancelled { ambiguous });
        }
        result
    }
}

fn response_error(status: u16, body: &[u8], ambiguous: bool) -> OperatorError {
    #[derive(serde::Deserialize)]
    struct Envelope {
        error: Detail,
    }
    #[derive(serde::Deserialize)]
    struct Detail {
        code: String,
        run_id: Option<String>,
    }
    if status == 403 {
        return OperatorError::Forbidden;
    }
    let Ok(envelope) = serde_json::from_slice::<Envelope>(body) else {
        return OperatorError::InvalidResponse { ambiguous };
    };
    if let Ok(code) = serde_json::from_value::<OperatorConnectionErrorCode>(
        serde_json::Value::String(envelope.error.code.clone()),
    ) {
        if status == code.status() {
            return OperatorError::Connection {
                code,
                ambiguous: ambiguous && status == 503,
            };
        }
        return OperatorError::InvalidResponse { ambiguous };
    }
    match (status, envelope.error.code.as_str()) {
        (400 | 413 | 415, "invalid") => OperatorError::Invalid,
        (404, "not_found") => OperatorError::NotFound,
        (409, "stale") => OperatorError::Stale,
        (409, "source_busy") => match envelope.error.run_id {
            Some(run_id) if uuid(&run_id).is_ok() => OperatorError::SourceBusy { run_id },
            _ => OperatorError::InvalidResponse { ambiguous },
        },
        (503, "unavailable") => OperatorError::Unavailable { ambiguous },
        _ => OperatorError::InvalidResponse { ambiguous },
    }
}
