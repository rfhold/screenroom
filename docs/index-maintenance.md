# Index maintenance

This guide owns poster repair, complete-generation publication, and the trusted-local operator boundary.
The [indexing architecture](indexing.md) owns canonical policies, queue progress, retry, redrive, and HTTP routes.
The [backend guide](backend.md#local-index-operator) owns lifecycle procedures.
The [Jellyfin guide](jellyfin.md) owns managed connection setup and private credentials.

## Web Maintenance

Web Maintenance supports [Jellyfin setup](jellyfin.md#web-setup), followed by canonical Refresh, Pause, Resume, and Retry controls.
Advanced settings expose saved policies, narrowed scope, run/candidate history, jobs, attempts, redrive history, and queue diagnostics.
Refresh uses an enabled saved policy; unsaved edits do not affect it.
Without saved settings, Refresh stays disabled; Add server or explicit Save settings creates the policy, never a configure-to-start continuation.
Native clients consume ordinary shared-core reads; they have no operator onboarding UI or direct Jellyfin HTTP.
The operator service supplies an API only, with no standalone dashboard or frontend assets.

Canonical indexing and repair replace the older poster-maintenance API.
There is no supported preview/start flow through `/api/sources/{source_id}/policies`, `/preview`, or `/start`.
Browser operation journals, recovery tabs, UUID exports, old-origin migration, and configure-to-start continuation are not supported.
The app does not read older valid or corrupt browser records. Page entry and reload do not replay them or authorize work.
Explicit in-session operations retain stable request UUIDs and exact bodies only in memory.
An ambiguous response offers explicit Retry of that same request, or reload to read server state; no automatic mutation retry exists.
Cancellation or page closure does not prove that the server stopped a mutation.

## Policy and scope

Canonical `repair` targets movie and series posters within one source.
It excludes metadata, embeddings, backdrops, logos, portraits, seasons, and episodes as repair targets.
The [policy matrix](indexing.md#policy-model) defines supported strategies, scope, effective budgets, and retry limits.
Manual scope only narrows the saved policy; unsaved edits do not alter a run's immutable snapshot.

With `skip_existing: true`, the planner excludes items with a complete, database-ready published poster generation.
A poster role label or unrelated fallback image does not satisfy that predicate.
Database readiness does not prove that remote object bytes remain healthy.
Canonical repair requires `skip_existing: true` exactly for `missing`; other strategies require false.

For fresh checks, the worker fetches provider artwork descriptors and verifies current objects.
It reuses healthy unchanged artwork; a check does not necessarily allocate a generation.
It discards unrelated provider metadata and performs no metadata mapping or embedding writes in poster repair.
The worker preserves the previous complete generation until every required replacement variant passes upload verification and reaches database-ready state.
Native artwork is mandatory for publication. Published revision URLs remain immutable.
Neither repair nor publication implements automatic object deletion or garbage collection.

## Status and controls

Canonical run history and candidate outcomes remain durable through the [operator API](indexing.md#canonical-operator-api).
Candidate records expose public/source-item selectors, kind, state, safe outcome code, selection reason, and timestamps.
They exclude provider external IDs, raw errors, credentials, private URLs, and job payloads.
The run summary has no `trigger` field.

Enumeration completion, settled work, and successful work remain different predicates.
`enumeration_complete` reports complete enumeration; `work_settled` reports no ready or leased jobs.
Terminal failure can settle work without success.
`work_succeeded` requires complete enumeration and run state, successful jobs/candidates, and no unresolved omissions, operations, attempts, or publications.
`all_complete` aliases successful work.
Historical failed receipts remain visible after logical resolution; a retry count alone does not establish current failure.
The [diagnostic contract](indexing.md#progress-completion-and-metrics) defines count overlap, non-atomic observations, and unknown worker liveness.

| State | Permitted run actions |
| --- | --- |
| `running` | `pause`, `cancel` |
| Eligible `paused` | `resume`, `cancel` |
| Other `paused` | `cancel` |
| Terminal | None |

The server's `allowed_actions` remains authoritative.
Pause revokes active leases through fences. Resume retains the same run and snapshots.
Cancel terminates ready/leased work and revokes fences.
Terminal job redrive uses separate eligibility; it neither resumes a paused run nor refunds budgets or deadlines.
The queue preserves attempts, request receipts, candidate progress, and publication provenance.

## Local operator API

The [operator protobuf](../proto/screenroom/operator/v1/operator.proto) defines the RPC surface.
`Health` checks the database; it does not verify provider reachability or worker activity.
`ListSources` and `ListLibraries` provide safe source and library discovery.
The [canonical RPC table](indexing.md#canonical-operator-api) covers policies, history, candidates, controls, jobs, attempts, redrive, and diagnostics.
The [connection API](jellyfin.md#connection-api) adds authenticated provider discovery and managed save.
Those connection operations contact Jellyfin; the operator is not a database-only service.

RPCs validate selectors, page bounds, and operation payloads. Errors contain safe codes, not raw upstream, SQL, or credential details.
Connection-specific safe codes appear in the [Jellyfin guide](jellyfin.md#errors-and-unsupported-actions).

### Web configuration

Catalog and `/maintenance` use the same configured backend. Maintenance needs no separate enable flag, listener, or proxy.
Pantheon serves the Web app and RPCs on one HTTPS origin.
For local development use the [Web configuration](../apps/web/README.md) and the intended instance descriptor.
Leaving the route unmounts setup and index controls, clearing keys, retry bodies, and polling.

### Browser boundary

Any caller that can reach the trusted private API can administer the instance. CORS and local user UUIDs do not authenticate callers.
Do not expose the API to an untrusted network.
WASM uses gRPC-Web, rejects redirects, and omits browser credentials. Domain payloads stay in Rust so wide integers retain precision.
Request payloads allow 65,536 bytes and response payloads allow 2 MiB.

## Source contracts

- [Repair repository](../crates/screenroom-backend/src/repair.rs): canonical candidate selection, status, and controls.
- [Artwork repository](../crates/screenroom-backend/src/media.rs): complete-generation publication.
- [Ingest repository](../crates/screenroom-backend/src/ingest.rs): durable progress, receipts, and fenced controls.
- [Queue administration](../crates/screenroom-backend/src/queue_admin.rs): attempts, diagnostics, and redrive eligibility.
- [Operator startup](../crates/screenroom-index-operator/src/main.rs): managed connections and listener construction.
- [Operator RPC adapter](../crates/screenroom-index-operator/src/rpc.rs): transport and sanitized errors.
