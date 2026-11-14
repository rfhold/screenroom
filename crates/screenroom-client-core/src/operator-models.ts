// Hand-maintained HTTP operator contract, separate from generated public RPC models.
export interface OperatorClientConfig { endpoint: string; timeout_ms: number }
export interface OperatorPage { after: string | null; limit: number }
export type OperatorItemKind = 'movie' | 'series';
export interface OperatorScope { library_ids: string[]; item_kinds: OperatorItemKind[]; item_ids: string[] }
export interface OperatorPosterPolicy { skip_existing: boolean }
export interface OperatorArtworkPolicy { posters: OperatorPosterPolicy }
export interface OperatorExistingPolicy { policy_id: string; expected_version: bigint }
export interface OperatorStartRequest { request_id: string; policy_id: string; policy_version: bigint; scope: OperatorScope | null }
export interface OperatorHealth { ready: boolean }
export interface OperatorSource { id: string; source_key: string; provider: string; state: string }
export interface OperatorLibrary { public_id: string; title: string }
export interface OperatorRun { id: string; source_id: string; policy_id: string; policy_version: bigint; state: string; items: bigint; requests: bigint; started_at: string; finished_at: string | null }
export type OperatorOutcomeCode = 'ready' | 'no_poster' | 'unavailable' | 'invalid' | 'unauthorized' | 'transport' | 'timeout';
export interface OperatorCandidate { source_item_id: string; public_id: string; kind: string; state: string; code: OperatorOutcomeCode | null; selection_reason: string | null; selected_at: string; updated_at: string }
export interface OperatorQueueCounts { ready: bigint; delayed: bigint; leased: bigint; failed: bigint }
export interface OperatorCandidateCounts { selected: bigint; checked: bigint; skipped_ready: bigint; staged: bigint; unavailable: bigint; failed: bigint }
export interface OperatorCoverage { unresolved_attempts: bigint; primary_unique: bigint; followup_ready: bigint; followup_delayed: bigint; followup_leased: bigint; followup_failed: bigint; unknown_attempts: bigint; retryable_attempts: bigint; permanent_attempts: bigint; omissions: bigint; unresolved_operations: bigint; pending_artwork_publications: bigint }
export interface OperatorRunStatus { observed_records: bigint | null; terminal_reason: string | null; run: OperatorRun; coverage: OperatorCoverage; queue: OperatorQueueCounts; candidates: OperatorCandidateCounts; allowed_actions: string[]; all_complete: boolean; enumeration_complete: boolean; work_settled: boolean; work_succeeded: boolean; as_of: string }
export interface OperatorSourcePage { items: OperatorSource[]; next_after: string | null }
export interface OperatorLibraryPage { items: OperatorLibrary[]; next_after: string | null }
export interface OperatorRunPage { items: OperatorRun[]; next_after: string | null }
export interface OperatorCandidatePage { items: OperatorCandidate[]; next_after: string | null }
export interface OperatorFailure extends Error { code: 'stale' | 'source_busy' | 'invalid' | 'not_found' | 'forbidden' | 'unavailable' | 'cancelled' | 'deadline' | 'invalid_response' | OperatorConnectionErrorCode; ambiguous: boolean; run_id?: string }
export type OperatorIndexingMode = 'full' | 'incremental' | 'reconcile' | 'repair';
export type OperatorIndexingStrategy = 'missing' | 'changed' | 'stale' | 'failed' | 'all';
export interface OperatorIndexingComponents { metadata: boolean; artwork: boolean; embeddings: boolean }
export interface OperatorRetryPolicy { max_attempts: number; base_seconds: number; cap_seconds: number; jitter_percent: number }
export interface OperatorIndexingLimits { work_requests: bigint; work_records: number; lifetime_requests: bigint; lifetime_records: bigint; deadline_seconds: number; work_timeout_seconds: number; retry: OperatorRetryPolicy }
export interface OperatorIndexingConfiguration { indexing: number; enabled: boolean; automatic_runs: boolean; cadence_seconds: number; reconcile_seconds: number; overlap_seconds: number; source_concurrency: number; mode: OperatorIndexingMode; components: OperatorIndexingComponents; strategy: OperatorIndexingStrategy; stale_after_seconds: number | null; scope: OperatorScope; limits: OperatorIndexingLimits; artwork: OperatorArtworkPolicy }
export interface OperatorIndexingConfigureRequest { request_id: string; existing: OperatorExistingPolicy | null; configuration: OperatorIndexingConfiguration }
export interface OperatorIndexingPolicy { id: string; source_id: string; version: bigint; enabled: boolean; automatic_runs: boolean; next_due_at: string; configuration: OperatorIndexingConfiguration }
export interface OperatorIndexingProgress { policy_version: bigint; watermark: string | null; next_reconcile_at: string }
export interface OperatorIndexingExecution { mode: OperatorIndexingMode; lower_watermark: string | null; candidate_watermark: string | null; settled_at: string | null; fallback_reason: string | null }
export type OperatorJobKind = 'ingest' | 'artwork' | 'embedding';
export type OperatorJobState = 'ready' | 'leased' | 'complete' | 'failed' | 'cancelled';
export type OperatorRedriveReason = 'not_terminal_failed' | 'not_current_terminal_job' | 'deadline_expired' | 'unsupported_payload';
export interface OperatorJob { id: string; run_id: string | null; kind: OperatorJobKind; state: OperatorJobState; available_at: string; lease_until: string | null; attempts: number; max_attempts: number; generation: number; redrive_allowed: boolean; redrive_reason: OperatorRedriveReason | null }
export type OperatorAttemptOutcome = 'running' | 'complete' | 'released' | 'permanent' | 'retryable' | 'throttled' | 'lease_expired' | 'exhausted' | 'paused' | 'cancelled';
export interface OperatorAttempt { job_id: string; fence: bigint; generation: number; started_at: string; finished_at: string | null; outcome: OperatorAttemptOutcome; error_code: string | null }
export interface OperatorRedriveRequest { request_id: string }
export interface OperatorRedrive { request_id: string; job_id: string; generation: number }
export interface OperatorDiagnosticsRequest { run_id: string | null; capabilities: string[] | null }
export type OperatorWorkerLiveness = 'unknown';
export interface OperatorQueueObservation { started_at: string; finished_at: string }
export interface OperatorQueueSnapshot { ready: bigint; delayed: bigint; leased: bigint; failed: bigint; cancelled: bigint; complete: bigint; claimable: bigint | null; reclaimable: bigint | null; blocked: bigint | null; exhausted: bigint; oldest_ready_seconds: number | null }
export interface OperatorQueueBlockers { deadline_expired: bigint; policy_disabled: bigint; policy_version: bigint; source_disabled: bigint; source_changed: bigint; source_concurrency: bigint; run_state: bigint; cooldown: bigint; capability: bigint | null; attempt_limit: bigint; next_eligible_at: string | null; as_of: string }
export interface OperatorCountSemantics { blocked: string; exhausted: string }
export interface OperatorQueueDiagnostics { atomic: false; queue_observation: OperatorQueueObservation; blockers_as_of: string; effective_capabilities: string[] | null; worker_liveness: OperatorWorkerLiveness; queue: OperatorQueueSnapshot; blockers: OperatorQueueBlockers; count_semantics: OperatorCountSemantics }
export interface OperatorIndexingPolicyPage { items: OperatorIndexingPolicy[]; next_after: string | null }
export interface OperatorJobPage { items: OperatorJob[]; next_after: string | null }
export interface OperatorAttemptPage { items: OperatorAttempt[]; next_after: string | null }
export interface OperatorRedrivePage { items: OperatorRedrive[]; next_after: string | null }
export interface OperatorDiscoverConnectionRequest { base_url: string; api_key: string }
export interface OperatorSaveConnectionRequest { request_id: string; base_url: string; api_key: string; server_id: string; profile_id: string; library_ids: string[]; local_user_id: string; refresh: boolean }
export interface OperatorDiscoveredServer { id: string; name: string; version: string }
export type OperatorDiscoveredLibraryKind = 'movies' | 'television' | 'collections' | 'unsupported';
export interface OperatorDiscoveredLibrary { id: string; name: string; kind: OperatorDiscoveredLibraryKind; supported: boolean }
export interface OperatorDiscoveredProfile { id: string; name: string; hidden: boolean }
export interface OperatorConnectionDiscovery { server: OperatorDiscoveredServer; libraries: OperatorDiscoveredLibrary[]; profiles: OperatorDiscoveredProfile[] }
export interface OperatorConnection { id: string; source_id: string; source_key: string; display_name: string; server_version: string; state: string }
export interface OperatorSavedConnection { connection: OperatorConnection; policy_id: string; policy_version: bigint; run_id: string | null }
export interface OperatorConnectionPage { items: OperatorConnection[]; next_after: string | null }
export type OperatorConnectionErrorCode = 'invalid_connection' | 'invalid_discovery' | 'invalid_credentials' | 'credentials_rejected' | 'invalid_server_identity' | 'invalid_selection' | 'profile_unavailable' | 'library_unavailable' | 'unsupported_library' | 'unsupported_library_scope' | 'selection_capacity' | 'local_user_unavailable' | 'request_conflict' | 'request_expired' | 'server_identity_mismatch' | 'server_already_connected' | 'connection_unavailable' | 'credential_store_unavailable' | 'discovery_unavailable';
