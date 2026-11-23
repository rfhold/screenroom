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

The [operator implementation](../crates/screenroom-index-operator/src/lib.rs) owns the HTTP routes and sanitized JSON envelopes.
`GET /api/health` checks the database; it does not verify provider reachability or worker activity.
`GET /api/sources` and `GET /api/sources/{source_id}/libraries` provide safe source/library discovery.
The [canonical route table](indexing.md#canonical-operator-api) owns policies, history, candidates, controls, jobs, attempts, redrive, and diagnostics.
The [connection API](jellyfin.md#connection-api) adds authenticated provider discovery and managed save.
Those connection operations contact Jellyfin; the operator is not a database-only service.

POST requests reject query parameters; GET requests reject bodies.
Typed bodies reject unknown fields. Errors contain safe codes, not raw upstream, SQL, or credential details.
The API returns HTTP 400 for invalid requests, 404 for unknown records, 409 for stale/source-busy conflicts, and 503 for unavailable dependencies.
Connection-specific safe codes appear in the [Jellyfin guide](jellyfin.md#errors-and-unsupported-actions).

### Explicit local attachment

Both server-only variables opt into the Web operator proxy:

| Variable | Example |
| --- | --- |
| `SCREENROOM_WEB_OPERATOR_TARGET` | `http://127.0.0.1:50073` |
| `SCREENROOM_WEB_OPERATOR_ORIGIN` | `http://127.0.0.1:5173` |

Neither has a default. Partial or invalid configuration fails Vite configuration.
Values require exact HTTP origins with numeric loopback hosts and no credentials, path, query, fragment, or trailing slash.
The target selects one fixed upstream; browser requests cannot supply another target.
Do not put the target in `VITE_` variables, URL parameters, or browser settings.

After separate approval for the Web restart, the development command is:

```sh
SCREENROOM_WEB_OPERATOR_TARGET=http://127.0.0.1:50073 \
  SCREENROOM_WEB_OPERATOR_ORIGIN=http://127.0.0.1:5173 \
  VITE_SCREENROOM_API_ENDPOINT=http://127.0.0.1:50052 \
  pnpm --filter @screenroom/web dev --host 127.0.0.1 --port 5173 --strictPort
```

The same pair must configure the production build and Vite preview.
These example ports attach the main development Web server to the local Jellyfin installation; confirm them against its descriptor.
Catalog and `/maintenance` share this origin and navigation. A second Web server is unnecessary.
Maintenance remains visible when the attachment is disabled and explains that it is unavailable.
Leaving the route unmounts setup and index controls, clearing keys, retry bodies, and polling.
Preview configuration cannot enable a build that lacks the capability; rebuild after attachment changes.
Public endpoint and local-user settings must also match the intended installation at development startup or production build.
Use `VITE_SCREENROOM_API_ENDPOINT` and the actual enabled user's `VITE_SCREENROOM_USER_ID`; Vite also accepts their `SCREENROOM_` aliases.
Explicit `VITE_` values take precedence over aliases. A missing or invalid user selector disables Add server submission.
Preview environment changes do not rewrite compiled public settings; rebuild when those settings change.
Static deployment alone supplies no proxy. The Web process starts no backend or worker.
These procedures do not start the operator or authorize provider acquisition, credential operations, or worker activation.

### Browser boundary

The opt-in origin grants trusted-local operator capability to every script on that origin, not just one screen.
The ordinary public `Client` and public RPC edge grant no operator authority.
The separate `OperatorClient` uses typed HTTP/JSON, not private RPC bindings.
Local user UUIDs, source selectors, and attachment configuration do not authenticate an administrator.

The [proxy](../apps/web/server/operator-proxy.mjs) serves development and preview through an exact `/__operator/api` allowlist and fixed `/api` upstream.
It checks loopback peers, exact single Host, Origin, Fetch Metadata, methods, query fields, content type, and body bounds.
POST requires the configured Origin and `application/json`.
The proxy creates upstream Host/Origin headers; it forwards no browser credentials or arbitrary headers.
It rejects redirects, bounds duration/responses, cancels upstream work on disconnect, and sanitizes errors.
Successful JSON bytes remain unchanged, so wide `i64` values avoid JavaScript number conversion.
The response limit is 2 MiB; request bodies allow 65,536 bytes.
Responses use `no-store` and `nosniff`.
These guards do not protect against another script on the trusted origin or authenticate native callers.

The direct operator listener requires exactly one Host header that matches its configured authority.
Any Origin header must match its configured origin; non-GET requests require Origin.
If present, `Sec-Fetch-Site` must equal `same-origin` or `none`.
POST requires exactly `Content-Type: application/json`, without a charset suffix.
The listener rejects encoded paths, backslashes, and dot segments.
Non-API paths, including `/`, `/index.html`, and `/assets/`, return 404.
Normal responses include `no-store`, `nosniff`, and this Content Security Policy:

```text
default-src 'self'; img-src 'self' data:; object-src 'none'; frame-ancestors 'none'; base-uri 'none'
```

Native callers can supply matching headers. Do not expose the service to an untrusted network.

## Source contracts

- [Repair repository](../crates/screenroom-backend/src/repair.rs): canonical candidate selection, status, and controls.
- [Artwork repository](../crates/screenroom-backend/src/media.rs): complete-generation publication.
- [Ingest repository](../crates/screenroom-backend/src/ingest.rs): durable progress, receipts, and fenced controls.
- [Queue administration](../crates/screenroom-backend/src/queue_admin.rs): attempts, diagnostics, and redrive eligibility.
- [Operator startup](../crates/screenroom-index-operator/src/main.rs): managed connections and listener construction.
- [Vite configuration](../apps/web/vite.config.ts): explicit local attachment.
