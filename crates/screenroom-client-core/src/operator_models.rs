use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorClientConfig {
    pub endpoint: String,
    pub timeout_ms: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorPage {
    pub after: Option<String>,
    pub limit: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Enum))]
pub enum OperatorItemKind {
    Movie,
    Series,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorScope {
    pub library_ids: Vec<String>,
    pub item_kinds: Vec<OperatorItemKind>,
    #[serde(default)]
    pub item_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorPosterPolicy {
    #[serde(default = "skip_existing_default")]
    pub skip_existing: bool,
}
fn skip_existing_default() -> bool {
    true
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorArtworkPolicy {
    pub posters: OperatorPosterPolicy,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorExistingPolicy {
    pub policy_id: String,
    pub expected_version: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorStartRequest {
    pub request_id: String,
    pub policy_id: String,
    pub policy_version: i64,
    pub scope: Option<OperatorScope>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorHealth {
    pub ready: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorSource {
    pub id: String,
    pub source_key: String,
    pub provider: String,
    pub state: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorLibrary {
    pub public_id: String,
    pub title: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorRun {
    pub id: String,
    pub source_id: String,
    pub policy_id: String,
    pub policy_version: i64,
    pub state: String,
    pub items: i64,
    pub requests: i64,
    pub started_at: String,
    pub finished_at: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorCandidate {
    pub selection_reason: Option<String>,
    pub source_item_id: String,
    pub public_id: String,
    pub kind: String,
    pub state: String,
    pub code: Option<OperatorOutcomeCode>,
    pub selected_at: String,
    pub updated_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Enum))]
pub enum OperatorOutcomeCode {
    Ready,
    NoPoster,
    Unavailable,
    Invalid,
    Unauthorized,
    Transport,
    Timeout,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorQueueCounts {
    pub ready: i64,
    pub delayed: i64,
    pub leased: i64,
    pub failed: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorCandidateCounts {
    pub selected: i64,
    pub checked: i64,
    pub skipped_ready: i64,
    pub staged: i64,
    pub unavailable: i64,
    pub failed: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorCoverage {
    pub unresolved_attempts: i64,
    pub primary_unique: i64,
    pub followup_ready: i64,
    pub followup_delayed: i64,
    pub followup_leased: i64,
    pub followup_failed: i64,
    pub unknown_attempts: i64,
    pub retryable_attempts: i64,
    pub permanent_attempts: i64,
    pub omissions: i64,
    pub unresolved_operations: i64,
    pub pending_artwork_publications: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorRunStatus {
    pub observed_records: Option<i64>,
    pub terminal_reason: Option<String>,
    pub run: OperatorRun,
    pub coverage: OperatorCoverage,
    pub queue: OperatorQueueCounts,
    pub candidates: OperatorCandidateCounts,
    pub allowed_actions: Vec<String>,
    pub all_complete: bool,
    pub enumeration_complete: bool,
    pub work_settled: bool,
    pub work_succeeded: bool,
    pub as_of: String,
}

macro_rules! page {
    ($name:ident, $item:ty) => {
        #[derive(Clone, Debug, Serialize, Deserialize)]
        #[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
        pub struct $name {
            pub items: Vec<$item>,
            pub next_after: Option<String>,
        }
    };
}
page!(OperatorSourcePage, OperatorSource);
page!(OperatorLibraryPage, OperatorLibrary);
page!(OperatorRunPage, OperatorRun);
page!(OperatorCandidatePage, OperatorCandidate);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Enum))]
pub enum OperatorIndexingMode {
    Full,
    Incremental,
    Reconcile,
    Repair,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Enum))]
pub enum OperatorIndexingStrategy {
    Missing,
    Changed,
    Stale,
    Failed,
    All,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorIndexingComponents {
    pub metadata: bool,
    pub artwork: bool,
    pub embeddings: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorRetryPolicy {
    pub max_attempts: i32,
    pub base_seconds: i32,
    pub cap_seconds: i32,
    pub jitter_percent: i32,
}
impl Default for OperatorRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_seconds: 5,
            cap_seconds: 3600,
            jitter_percent: 25,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorIndexingLimits {
    pub work_requests: i64,
    pub work_records: u32,
    pub lifetime_requests: i64,
    pub lifetime_records: i64,
    pub deadline_seconds: i32,
    pub work_timeout_seconds: i32,
    #[serde(default)]
    pub retry: OperatorRetryPolicy,
}
fn reconcile_default() -> i32 {
    604800
}
fn overlap_default() -> i32 {
    300
}
fn concurrency_default() -> u32 {
    1
}
fn artwork_default() -> OperatorArtworkPolicy {
    OperatorArtworkPolicy {
        posters: OperatorPosterPolicy {
            skip_existing: true,
        },
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorIndexingConfiguration {
    pub indexing: u32,
    pub enabled: bool,
    #[serde(default)]
    pub automatic_runs: bool,
    pub cadence_seconds: i32,
    #[serde(default = "reconcile_default")]
    pub reconcile_seconds: i32,
    #[serde(default = "overlap_default")]
    pub overlap_seconds: i32,
    #[serde(default = "concurrency_default")]
    pub source_concurrency: u32,
    pub mode: OperatorIndexingMode,
    pub components: OperatorIndexingComponents,
    pub strategy: OperatorIndexingStrategy,
    pub stale_after_seconds: Option<i32>,
    pub scope: OperatorScope,
    pub limits: OperatorIndexingLimits,
    #[serde(default = "artwork_default")]
    pub artwork: OperatorArtworkPolicy,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorIndexingConfigureRequest {
    pub request_id: String,
    pub existing: Option<OperatorExistingPolicy>,
    pub configuration: OperatorIndexingConfiguration,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorIndexingPolicy {
    pub id: String,
    pub source_id: String,
    pub version: i64,
    pub enabled: bool,
    pub automatic_runs: bool,
    pub next_due_at: String,
    pub configuration: OperatorIndexingConfiguration,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorIndexingProgress {
    pub policy_version: i64,
    pub watermark: Option<String>,
    pub next_reconcile_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorIndexingExecution {
    pub mode: OperatorIndexingMode,
    pub lower_watermark: Option<String>,
    pub candidate_watermark: Option<String>,
    pub settled_at: Option<String>,
    pub fallback_reason: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Enum))]
pub enum OperatorJobKind {
    Ingest,
    Artwork,
    Embedding,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Enum))]
pub enum OperatorJobState {
    Ready,
    Leased,
    Complete,
    Failed,
    Cancelled,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Enum))]
pub enum OperatorRedriveReason {
    NotTerminalFailed,
    NotCurrentTerminalJob,
    DeadlineExpired,
    UnsupportedPayload,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorJob {
    pub id: String,
    pub run_id: Option<String>,
    pub kind: OperatorJobKind,
    pub state: OperatorJobState,
    pub available_at: String,
    pub lease_until: Option<String>,
    pub attempts: i32,
    pub max_attempts: i32,
    pub generation: i32,
    pub redrive_allowed: bool,
    pub redrive_reason: Option<OperatorRedriveReason>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Enum))]
pub enum OperatorAttemptOutcome {
    Running,
    Complete,
    Released,
    Permanent,
    Retryable,
    Throttled,
    LeaseExpired,
    Exhausted,
    Paused,
    Cancelled,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorAttempt {
    pub job_id: String,
    pub fence: i64,
    pub generation: i32,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub outcome: OperatorAttemptOutcome,
    pub error_code: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorRedriveRequest {
    pub request_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorRedrive {
    pub request_id: String,
    pub job_id: String,
    pub generation: i32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorDiagnosticsRequest {
    pub run_id: Option<String>,
    pub capabilities: Option<Vec<String>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Enum))]
pub enum OperatorWorkerLiveness {
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorQueueObservation {
    pub started_at: String,
    pub finished_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorQueueSnapshot {
    pub ready: i64,
    pub delayed: i64,
    pub leased: i64,
    pub failed: i64,
    pub cancelled: i64,
    pub complete: i64,
    pub claimable: Option<i64>,
    pub reclaimable: Option<i64>,
    pub blocked: Option<i64>,
    pub exhausted: i64,
    pub oldest_ready_seconds: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorQueueBlockers {
    pub deadline_expired: i64,
    pub policy_disabled: i64,
    pub policy_version: i64,
    pub source_disabled: i64,
    pub source_changed: i64,
    pub source_concurrency: i64,
    pub run_state: i64,
    pub cooldown: i64,
    pub capability: Option<i64>,
    pub attempt_limit: i64,
    pub next_eligible_at: Option<String>,
    pub as_of: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorCountSemantics {
    pub blocked: String,
    pub exhausted: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorQueueDiagnostics {
    pub atomic: bool,
    pub queue_observation: OperatorQueueObservation,
    pub blockers_as_of: String,
    pub effective_capabilities: Option<Vec<String>>,
    pub worker_liveness: OperatorWorkerLiveness,
    pub queue: OperatorQueueSnapshot,
    pub blockers: OperatorQueueBlockers,
    pub count_semantics: OperatorCountSemantics,
}
page!(OperatorIndexingPolicyPage, OperatorIndexingPolicy);
page!(OperatorJobPage, OperatorJob);
page!(OperatorAttemptPage, OperatorAttempt);
page!(OperatorRedrivePage, OperatorRedrive);

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorDiscoverConnectionRequest {
    pub base_url: String,
    pub api_key: String,
}
impl std::fmt::Debug for OperatorDiscoverConnectionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OperatorDiscoverConnectionRequest([REDACTED])")
    }
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorSaveConnectionRequest {
    pub request_id: String,
    pub base_url: String,
    pub api_key: String,
    pub server_id: String,
    pub profile_id: String,
    pub library_ids: Vec<String>,
    pub local_user_id: String,
    #[serde(default)]
    pub refresh: bool,
}
impl std::fmt::Debug for OperatorSaveConnectionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OperatorSaveConnectionRequest([REDACTED])")
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorDiscoveredServer {
    pub id: String,
    pub name: String,
    pub version: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Enum))]
pub enum OperatorDiscoveredLibraryKind {
    Movies,
    Television,
    Collections,
    Unsupported,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorDiscoveredLibrary {
    pub id: String,
    pub name: String,
    pub kind: OperatorDiscoveredLibraryKind,
    pub supported: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorDiscoveredProfile {
    pub id: String,
    pub name: String,
    pub hidden: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorConnectionDiscovery {
    pub server: OperatorDiscoveredServer,
    pub libraries: Vec<OperatorDiscoveredLibrary>,
    pub profiles: Vec<OperatorDiscoveredProfile>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorConnection {
    pub id: String,
    pub source_id: String,
    pub source_key: String,
    pub display_name: String,
    pub server_version: String,
    pub state: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct OperatorSavedConnection {
    pub connection: OperatorConnection,
    pub policy_id: String,
    pub policy_version: i64,
    pub run_id: Option<String>,
}
page!(OperatorConnectionPage, OperatorConnection);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Enum))]
pub enum OperatorConnectionErrorCode {
    InvalidConnection,
    InvalidDiscovery,
    InvalidCredentials,
    CredentialsRejected,
    InvalidServerIdentity,
    InvalidSelection,
    ProfileUnavailable,
    LibraryUnavailable,
    UnsupportedLibrary,
    UnsupportedLibraryScope,
    SelectionCapacity,
    LocalUserUnavailable,
    RequestConflict,
    RequestExpired,
    ServerIdentityMismatch,
    ServerAlreadyConnected,
    ConnectionUnavailable,
    CredentialStoreUnavailable,
    DiscoveryUnavailable,
}
impl OperatorConnectionErrorCode {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::InvalidConnection => "invalid_connection",
            Self::InvalidDiscovery => "invalid_discovery",
            Self::InvalidCredentials => "invalid_credentials",
            Self::CredentialsRejected => "credentials_rejected",
            Self::InvalidServerIdentity => "invalid_server_identity",
            Self::InvalidSelection => "invalid_selection",
            Self::ProfileUnavailable => "profile_unavailable",
            Self::LibraryUnavailable => "library_unavailable",
            Self::UnsupportedLibrary => "unsupported_library",
            Self::UnsupportedLibraryScope => "unsupported_library_scope",
            Self::SelectionCapacity => "selection_capacity",
            Self::LocalUserUnavailable => "local_user_unavailable",
            Self::RequestConflict => "request_conflict",
            Self::RequestExpired => "request_expired",
            Self::ServerIdentityMismatch => "server_identity_mismatch",
            Self::ServerAlreadyConnected => "server_already_connected",
            Self::ConnectionUnavailable => "connection_unavailable",
            Self::CredentialStoreUnavailable => "credential_store_unavailable",
            Self::DiscoveryUnavailable => "discovery_unavailable",
        }
    }
    pub(crate) fn status(&self) -> tonic::Code {
        match self {
            Self::RequestConflict
            | Self::RequestExpired
            | Self::ServerIdentityMismatch
            | Self::ServerAlreadyConnected => tonic::Code::AlreadyExists,
            Self::ConnectionUnavailable
            | Self::CredentialStoreUnavailable
            | Self::DiscoveryUnavailable => tonic::Code::Unavailable,
            _ => tonic::Code::InvalidArgument,
        }
    }
}
impl std::fmt::Display for OperatorConnectionErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
