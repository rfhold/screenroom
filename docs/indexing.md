# Indexing architecture

This guide owns the implemented provider-neutral queue, backend policy, recovery, incremental acquisition, and status contracts.
The [backend guide](backend.md) owns runtime lifecycle and migration procedures.
The [maintenance guide](index-maintenance.md) owns poster semantics, operator security, and Web opt-in.
The [Jellyfin guide](jellyfin.md) owns managed setup and private credential ownership.
The [client guide](client-api.md#operator-capability) owns the separate shared operator capability.

## Implementation boundary

All indexing execution paths use one PostgreSQL queue abstraction.
Canonical backend policies and operator HTTP routes support full, incremental, reconcile, and repair modes within the limits below.
Web Maintenance supports Jellyfin setup and canonical Refresh, Pause, Resume, and Retry through the separate [shared operator capability](client-api.md#operator-capability).
Refresh requires a saved enabled policy; no-policy Refresh never creates settings or starts a configure-to-start chain.
Native operator UI remains absent. Browser operation journals and old maintenance routes are not supported.
Per-operation worker transport metrics cover provider, embedding, and storage I/O with the consumption semantics below.
Source support does not establish migration application, worker activation, or live provider acquisition.

Source entry points include:

- [Shared queue](../crates/screenroom-queue/src/lib.rs): transactional transport, retry, attempts, and redrive receipts.
- [Ingest repository](../crates/screenroom-backend/src/ingest.rs): durable jobs, run snapshots, receipts, leases, and fences.
- [Indexing repository](../crates/screenroom-backend/src/indexing.rs): canonical policy validation, source admission, execution windows, and settlement.
- [Repair repository](../crates/screenroom-backend/src/repair.rs): poster candidate selection and status.
- [Queue administration](../crates/screenroom-backend/src/queue_admin.rs): diagnostic counts, history, and domain redrive eligibility.
- [Worker pipeline](../crates/screenroom-index-worker/src/pipeline.rs) and [full traversal](../crates/screenroom-index-worker/src/full.rs): capability-aware execution and incremental acquisition.
- [Jellyfin delta adapter](../crates/screenroom-catalog-adapter/src/jellyfin/delta.rs): timestamp filters and capability fallback.
- [Operator service](../crates/screenroom-index-operator/src/lib.rs): local HTTP/JSON routes.

## Queue and domain responsibilities

One provider-neutral PostgreSQL queue serves full traversal, incremental acquisition, reconciliation, repair, metadata, artwork, and embeddings.
The queue owns transactional enqueue, atomic claims, leases, fence tokens, attempt records, retry scheduling, and terminal state.
The domain owns eligibility, source cooldowns, catalog transactions, traversal validity, and publication predicates.
Provider adapters own provider capabilities and safe failure classification, not private queue implementations.

Workers retain no authoritative progress in process memory.
Durable checkpoints, immutable policy snapshots, traversal scope, and incremental windows permit another compatible worker to continue after process loss.
Jobs declare required capabilities; claims select compatible workers.
Runs that require embeddings pin an immutable model identity; jobs require the matching model capability.
Embedding follow-ups also bind the current document input hash; stale content cannot publish a vector for another input.
Canonical metadata-only and poster repair runs do not require an embedding model.
Worker capabilities include adapter, source, private locator revision, and model tokens as applicable.
Domain admission also checks policy version, source configuration, run state, deadline, source concurrency, and cooldown.

Queue delivery provides at-least-once execution, not exactly-once external effects.
Domain transactions and idempotent effects prevent duplicate delivery from duplicate publication or catalog changes.
Transactions couple durable domain progress with follow-up enqueue wherever both share PostgreSQL.
External operations retain durable receipts and ambiguity diagnostics across interruption.
Lease expiry permits recovery; fences reject stale workers before authoritative commits.

Active transport deduplication uses queue, run scope, kind, and dedupe key; identical ownership also requires equal input and capabilities.
Each run retains its own publication jobs and completion evidence.
One run's pause or cancellation does not orphan another run's work.
Object bytes use separate content-addressed deduplication under source scope.

## Retry and recovery

One classified retry policy applies across queue handlers.
Retryable failures use bounded exponential backoff with jitter and provider `Retry-After` guidance.
Permanent failures and exhausted attempts enter a durable terminal state.
Effective deadlines, attempt limits, timeouts, source concurrency, and budgets bound execution; retries do not refund consumed budgets.
Proactive request-rate configuration and source concurrency above one are unsupported.
Source cooldowns participate in domain eligibility and remain visible even when raw queue state says ready.

Each attempt retains its disposition and safe diagnostic evidence.
Current unresolved failures remain distinct from historical failed attempts after successful recovery.
Status never treats a historical retry count alone as proof of current failure.
Unknown external outcomes retain their receipt provenance.
Logical work-unit resolutions explain successful replay or full-scan fallback without deletion or alteration of historical errors.

Three operator actions have different meanings:

| Action | Contract |
| --- | --- |
| Resume | Continue a paused run with its durable progress and immutable snapshots. |
| Start replay | Reconcile an ambiguous start through the identical request UUID and body; return the original run. |
| Terminal redrive | Explicitly requeue an eligible failed job under the same identity, with a new generation and durable request receipt. |

Redrive does not masquerade as resume or allocate an unrelated start request.
The server validates current policy, source, run, deadline, and payload before redrive.
Redrive preserves attempt history, prior run coverage, request charges, work-window charges, and checkpoint progress.
An identical redrive request UUID returns its original receipt; reuse for another job conflicts.
Cancelled, obsolete, superseded, expired, and incompatible work does not qualify.
If traversal failed, its ingest job requires redrive before follow-ups regain eligibility.
The [canonical operator API](#canonical-operator-api) exposes job eligibility and redrive history independently of run resume actions.

## Policy model

The backend validates `indexing: 1` through `indexing::Configuration`.
Canonical policies require an enabled managed Jellyfin source with a private connection/profile binding and valid source revision.
The queue remains provider-neutral; canonical policy admission does not imply live support for every adapter.

| Dimension | Supported combinations |
| --- | --- |
| `full`, `incremental`, `reconcile` | Metadata required; artwork and embeddings optional; strategy `all`; library scope with both movie and series kinds; no item IDs. |
| `repair` | Posters only: metadata and embeddings false, artwork true; strategy `missing`, `changed`, `stale`, `failed`, or `all`. |
| Repair scope | Selected libraries, movie/series kinds, and optional item IDs; manual scope only narrows the saved scope. |
| Artwork reuse | `skip_existing: true` for traversal; repair requires true exactly for `missing`, false for every other strategy. |
| Stale repair | Required `stale_after_seconds` from 60 through 2,592,000; other strategies reject this field's non-null value. |

Policy validation rejects unsupported combinations and unknown fields.
A stored field without an effective execution path is not a supported capability.
Manual scope narrows a saved policy; it does not silently alter its target or execution limits.
Runs persist effective policy and selection snapshots so later edits cannot change work already in progress.

Repair `changed` compares saved provider poster descriptors with published generation inputs, not fresh provider metadata.
Repair `stale` uses the latest successful complete check or publication age; unknown age yields `selection_reason: unknown_age`.
Repair `failed` selects latest failed candidate outcomes or a current pending generation's failed publication job, not every historical failure.

### Defaults and limits

| Field | Default or accepted range |
| --- | --- |
| `automatic_runs` | `false`; both enabled settings and a due policy are necessary for automatic work. |
| `overlap_seconds` | 300; accepts 1–86,400. |
| `reconcile_seconds` | 604,800; accepts 60–2,592,000. |
| `source_concurrency` | 1; rejects every other value. |
| `cadence_seconds` | Required; accepts 60–2,592,000. |
| `limits.work_requests` | Required; accepts 7–1,000. |
| `limits.work_records` | Required; accepts 3–1,000. |
| `limits.lifetime_requests`, `limits.lifetime_records` | Required; each accepts 1–100,000,000. |
| `limits.deadline_seconds` | Required; accepts 60–2,592,000 from run start. |
| `limits.work_timeout_seconds` | Required; accepts 1–240 per worker step. |
| `limits.retry.max_attempts` | 5; accepts 1–20. |
| `limits.retry.base_seconds` | 5; accepts 1–300. |
| `limits.retry.cap_seconds` | 3,600; accepts the base through 86,400. |
| `limits.retry.jitter_percent` | 25; accepts 0–100. |

The retry object defaults as a whole; a supplied object requires all four fields.
Retry delay combines capped exponential backoff, stable per-attempt jitter, and bounded `Retry-After` guidance.
The maximum delay is 86,400 seconds; a provider delay can exceed the exponential cap.
Successful durable progress resets the consecutive-failure tranche and work request window, not lifetime charges or attempt history.
Provider calls also retain adapter request limits and full traversal's ten-second request bound.

Per-work limits bound each durable work window; run lifetime limits cover the entire run across leases, retries, resume, and redrive.
`observed_records` charges distinct item IDs within each commit, including unchanged metadata touches; repeated observations across commits still consume budget.
`run.items` counts metadata writes, not all observations, and cannot establish lifetime record headroom.
Lifetime requests count durable reservations before I/O, not exact wire calls; an interrupted reservation still consumes budget.
The worker checks deadlines at durable admission, reservation, commit, and settlement boundaries.
Deadline expiry fails the run, cancels active jobs, revokes fences, and prohibits redrive; it does not promise instantaneous cancellation of external I/O.

The canonical repair `skip_existing` predicate remains in the [maintenance guide](index-maintenance.md#policy-and-scope).
It requires a complete database-ready published poster generation, not a role label or unrelated fallback image.
The broader model does not weaken complete-generation publication or preservation of the previous complete image.

## Source provisioning and locator binding

The [managed connection repository](../crates/screenroom-backend/src/connections.rs) provisions the source, revision, selected libraries, and private profile binding atomically.
The private revision binds managed connection identity, credential reference, authenticated server identity, and normalized URL.
The source also records the connection generation. Public operator records exclude these private inputs and the revision.
A digest does not establish authentication; runtime provider resolution separately checks the actual ServerId.
Canonical runs snapshot the source configuration; later binding changes block old work and settlement.
Workers obtain current source/revision capabilities from managed database records without restart or manual enrollment.
User-state acquisition uses the same private connection store, not browser credentials or provider JSON.
The [Jellyfin contract](jellyfin.md#credential-ownership) owns file access and credential privacy.

The worker CLI accepts `serve`, `drain`, and `activate-model` only.
Configure, start, pause, resume, cancel, and redrive use canonical operator routes.
Old sample/index initializers and separate enrollment commands are not supported product interfaces.
Code availability grants no provider-work or lifecycle authority.

## Progress, completion, and metrics

Enumeration finish and settled work are separate conditions.
A run can exhaust enumeration while queue work, publication, or unresolved outcomes still prevent successful settlement and watermark advancement.
Status exposes semantic availability alongside raw queue states:

| Status concept | Meaning |
| --- | --- |
| `ready`, `delayed` | Raw ready-state jobs, partitioned by availability time; neither establishes domain eligibility. |
| `claimable` | Due ready jobs within attempt limits that pass domain and supplied capability checks. |
| `reclaimable` | Expired leased jobs within attempt limits that pass the same checks; excludes `claimable`. |
| `blocked` | Due ready or expired leased jobs with domain or capability blockers; attempt limit alone does not qualify. |
| `exhausted` | Ready or expired leased jobs at the attempt limit, including delayed ready jobs. |
| Terminal | Failed, cancelled, or complete queue state; only eligible failed jobs support redrive. |

Counts distinguish current unresolved failures from historical attempt failures.
Progress survives restarts and includes durable traversal snapshots and incremental windows.
Status explains blockers instead of equating a raw ready count with executable work.
`blocked` and `exhausted` overlap when a job has another blocker; blocker reasons also overlap.
The oldest-ready age measures raw due-ready age, not oldest claimable age.
Without explicit capabilities, HTTP diagnostics return null claimable, reclaimable, blocked, and capability-blocker counts.
An explicit empty capability list differs from an unknown worker context.
Diagnostics report `atomic: false`, a queue observation interval, and `blockers_as_of`; those observations do not share an atomic snapshot.
`worker_liveness: unknown` means that no heartbeat establishes worker health; queue counts are not a liveness signal.

Run status exposes `enumeration_complete`, `work_settled`, and `work_succeeded`; `all_complete` aliases successful work.
Settled work has no ready or leased jobs, but can still contain terminal failures.
Successful work additionally requires complete enumeration, no failed/cancelled jobs, and no unresolved candidates, omissions, operations, attempts, or publications.
Historical receipt failures remain visible after logical resolution and do not alone prevent successful work.
The [maintenance completion predicates](index-maintenance.md#status-and-controls) share this distinction.

Queue and worker instruments cover queue observations, transitions, processing duration, safe error classes, and transport resource observations.
Metric labels use bounded enums for job kind, transition, outcome, safe failure class, request kind, and body direction.
The worker attaches a per-operation `JobTransport` observer to provider, embedding, and object-storage clients.
Provider JSON, capability checks, delta pages, artwork, and embedding requests share that operation's job-kind context.
Storage PUT, HEAD, and verification GET requests retain separate request kinds.
Body observations include consumed download chunks and upload chunks that the HTTP body consumer requests, including partial failures and cancellation.
They do not measure wire bytes or prove server receipt, acknowledgement, or durable storage.
`request.kind` and `body.direction` distinguish request families and consumption direction; request reservations remain a separate budget measure.
Transport success means accepted HTTP status and required body consumption, not successful decode, validation, indexing, or publication.
An unfinished request guard reports cancellation once; completed request guards retain their transport outcome.
Optional OTLP configuration does not establish collector delivery or exporter-to-collector verification.
UUIDs, run IDs, item IDs, arbitrary source keys, URLs, and raw error messages do not belong in metric labels.
Logs and traces carry appropriate correlation identifiers under their own privacy rules.
The [current telemetry guide](backend.md#shared-telemetry) describes the actual exporter boundary.

## Canonical operator API

These routes use the [local operator boundary](index-maintenance.md#local-operator-api) and its security/body limits.
They read or mutate PostgreSQL; provider discovery/save uses the separate [connection API](jellyfin.md#connection-api).
Web Maintenance consumes these contracts through shared operator methods and the guarded proxy.
Refresh uses canonical saved policies, not the removed poster preview/start API.
Explicit Save and Refresh can atomically create a connection and initial run; Save alone creates no jobs.
Page entry, reload, and settings reads do not authorize work or replay historical browser records.

| Method and path | Contract |
| --- | --- |
| `GET /api/indexing/sources/{source_id}/policies` | Sanitized canonical policies. |
| `POST /api/indexing/sources/{source_id}/policies` | `{request_id, existing, configuration}`; versioned, idempotent configure. |
| `POST /api/indexing/sources/{source_id}/start` | `{request_id, policy_id, policy_version, scope}`; sanitized run summary with identical-request replay. |
| `GET /api/indexing/sources/{source_id}/policies/{policy_id}/progress` | Durable policy watermark and next reconciliation time. |
| `GET /api/indexing/sources/{source_id}/runs` | Canonical source run history; UUID cursor. |
| `GET /api/indexing/sources/{source_id}/runs/{run_id}` | Shared sanitized run status and allowed run actions. |
| `GET /api/indexing/sources/{source_id}/runs/{run_id}/candidates` | Sanitized candidate outcomes; UUID cursor and optional `pending_only`. |
| `GET /api/indexing/sources/{source_id}/runs/{run_id}/execution` | Actual mode, window, fallback reason, and settlement time. |
| `POST /api/indexing/sources/{source_id}/runs/{run_id}/{pause,resume,cancel}` | Empty object; permitted run action, not redrive. |
| `GET /api/indexing/sources/{source_id}/runs/{run_id}/jobs` | Payload-free job summaries with `redrive_allowed` and `redrive_reason`. |
| `GET /api/indexing/sources/{source_id}/runs/{run_id}/jobs/{job_id}/attempts` | Attempt history without lease-owner strings; numeric fence cursor. |
| `GET /api/indexing/sources/{source_id}/runs/{run_id}/jobs/{job_id}/redrives` | Redrive history; numeric generation cursor. |
| `POST /api/indexing/sources/{source_id}/runs/{run_id}/jobs/{job_id}/redrive` | `{request_id}`; idempotent terminal redrive receipt. |
| `POST /api/indexing/queue` | `{run_id, capabilities}`; non-atomic diagnostic observations and blocker reasons. |

Policy, job, attempt, and redrive collections accept limits from 1 through 200; the default is 100.
Run/candidate history accepts limits from 1 through 1,000; candidate `pending_only=true` selects `selected` and `checked` outcomes.
UUID cursors apply to policies and jobs; attempt and redrive cursors start after zero by default.
Canonical responses exclude raw run snapshots, job input, raw policy records, provider locators, credentials, and `operator_profile_id`.
Configure rejects browser-supplied `operator_profile_id`.
The server resolves the enabled source's managed connection profile and injects it before backend validation.
Configure cannot bootstrap an unbound source; managed connection Save supplies that boundary.
This binding selects a local profile; it does not authenticate an administrator.
Run summaries omit `trigger`; old maintenance route families are not compatibility interfaces.

## Jellyfin incremental acquisition

Jellyfin metadata deltas use bounded timestamp polls with overlap, durable checkpoints, and explicit capability detection.
They do not replace complete reconciliation or create an authoritative event journal.
The durable window records scope, the prior checkpoint, the overlap lower bound, and the next checkpoint candidate.
Only fully settled successful whole-policy work permits watermark advancement.
Interrupted or incomplete polls retain the previous checkpoint; idempotent replay tolerates overlap.
An item creation timestamp does not substitute for a metadata modification watermark.

The adapter admits stable three-component Jellyfin 10.11.x versions only; unknown versions and prereleases require a full scan.
It checks capability for every delta page and falls back explicitly when the filter rejects the request.
Source inspection identified an Items-filter SQL parameter defect in Jellyfin 10.10.7; no live reproduction establishes its runtime scope here.
That finding does not prove that all Jellyfin 10.10 releases fail.
Unknown or incompatible capability must produce explicit fallback, not silent incremental success.

An incremental policy starts with reconciliation when its watermark is absent or periodic reconciliation is due.
The worker persists timestamp windows, page checkpoints, fallback state, ancestry, and bounded changed-series/season descendant refresh progress.
Collections use complete traversal rather than timestamp delta filters.
Normalized metadata and document hashes avoid redundant writes; unchanged items still refresh observations and consume record budget.
Successful artwork reuse and hash/model-matched embeddings avoid unnecessary follow-up effects.

The adapter uses `MinDateLastSaved` as a lower bound; the window's upper candidate is not an upstream snapshot filter.
Offset pages and sort ties can still omit records under concurrent changes, even when duplicate and total guards pass.
Overlap and periodic complete reconciliation reduce omissions; they do not provide a journal or snapshot guarantee.
Narrowed manual runs cannot advance the saved policy's whole-scope watermark.
Settlement also requires current policy/source configuration, every queue job complete, and no unresolved outcomes, omissions, operations, or publications.

Live full, reconcile, and incremental traversal remains non-authoritative: completion does not mark absent items or remove memberships.
Generic ingest completion grants no absence authority and does not automatically disable the policy.
Partial traversal, timestamp-poll absence, failures, and budget stops never authorize deletions.
Mutable execution requires validated per-work budgets and canonical admission; historical lifetime checkpoint decoding provides read-only interpretation only.

`LibraryChanged` notifications remain optional future acquisition hints; no notification consumer forms part of this execution contract.
A Kodi-style plugin remains an optional future integration, not a prerequisite for the timestamp-poll design.
Personal [watched snapshots](backend.md#watched-snapshots) remain separate from catalog metadata deltas and search documents.
Browsers consume provider-neutral operator records and actions; they do not implement Jellyfin traversal or capability rules.

## Hard cutover and activation boundary

The code cutover uses the shared queue directly; it provides no legacy queue compatibility view or parallel legacy executor.
Forward-only migrations preserve catalog data, artwork, embeddings, users, identity links, installation identity, and receipt provenance.
Applied migrations `001` through `019` remain immutable.
Forward-only migrations `010` through `019` implement the queue cutover, canonical policies, watermarks, admission, age, records, resolutions, reasons, locators, and deadlines.
Migration `010` moves the job table into `queue`, preserves job identities, and repairs active cross-run publication dependencies.
Later migrations retain durable receipts and append logical resolutions rather than erase failure provenance.
Migration files establish source availability, not the applied schema version of any persistent instance.
The [migration guide](backend.md#database-startup-and-migrations) owns the actual startup procedure.
Migration `020` drops only the obsolete exclusive `ingest.maintenance_starts` table without `CASCADE`.
Migrations `021` and `022` add managed connections and restricted private read grants.
Canonical queue/run receipts and their provenance remain durable.
Older browser journals receive no replay, export, migration, or continuation support; current clients ignore those records.
The operator API serves no standalone dashboard.

Local code and documentation changes grant no live activation authority.
Service restarts, migration application, worker activation, and provider acquisition require separate explicit approval for the exact target.
Destructive commands require separate approval; this guide supplies no reset or volume-removal procedure.
The operator remains an explicit loopback-only opt-in capability with no administrator authentication.
The [browser security rules](index-maintenance.md#browser-boundary) retain the trusted-local, no-authentication boundary.
