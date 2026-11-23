# Backend development

The catalog, user, playback gateway, and index worker use shared PostgreSQL repositories.
Managed Jellyfin connections supply real provider data; runtime fixture adapters and backend profiles are not supported.
The [component outline](components.md) owns intended product scope and the public deployment contract.
The [client API guide](client-api.md) owns the public client contract, separate operator capability, and local settings boundary.
The [indexing architecture](indexing.md) owns the shared queue, retry/redrive, supported policies, incremental execution, and diagnostic semantics.
Implemented source does not establish active runtime capabilities or authorize lifecycle changes.

## Transport boundary

| Executable | Registered services | Business behavior |
| --- | --- | --- |
| `screenroom-catalog` | `screenroom.catalog.v1.BrowseService`, `screenroom.catalog.v1.CatalogService`, and `screenroom.catalog.v1.SearchService` | Persistent scoped catalog, search, artwork credits, and artwork HTTP transport. |
| `screenroom-user` | `screenroom.user.v1.UserService` | Local user reads and validated Jellyfin profile links through UUID metadata. |
| `screenroom-playback-gateway` | `screenroom.playback_gateway.v1.PlaybackService` and `/playback/v1/media/` | Durable Jellyfin sessions, private device authorization, media transport, personal reports, and cleanup. |
| `screenroom-index-operator` | Separate local HTTP/JSON API | Managed Jellyfin discovery/save and canonical policies, diagnostics, history, redrive, and run controls; no frontend hosting. |

The catalog, user, and playback gateway accept HTTP/2 gRPC and HTTP/1 gRPC-Web through `tonic-web` on one listener.
Envoy routes public catalog, user, and playback paths through one loopback edge.
It does not expose operator or internal playback-state RPCs.
No service implements authentication or JWT verification.

Local user UUIDs select state; they are not credentials. Private listeners provide network separation, not authentication or administrator validation.
Do not expose the local stack to an untrusted network.

## Persistent local instance

Backend startup requires explicit approval, Docker Compose, Python 3, grpcurl, and Buf for native/browser readiness checks.
An embedding endpoint is optional; a zero-source installation and empty catalog are healthy.
Startup builds the backend image unless `SCREENROOM_BACKEND_BUILD=0` selects a current local image.
From the repository root:

```sh
bash scripts/backend.sh up
bash scripts/backend.sh wait
bash scripts/backend.sh status
bash scripts/backend.sh down
```

These commands mutate local runtime state; they are not prose-validation commands.
`up` initializes missing installation files, private Garage keys, private credential volumes, and a local user before service startup.
Its create-only user initialization preserves operator names, disabled users, cleared links, and remapped links.
The explicit user initializer supports ordinary local user updates, not provider-link imports; service startup itself does not create users.
`wait` compares native/browser catalog information and user RPCs with persisted installation identity.
It also checks the private playback `/ready` endpoint before it writes `checks.playback=true` in the descriptor.
Readiness does not require provider content, artwork, complete semantic coverage, or an embedding endpoint.
HTTP 200 alone does not establish readiness. `status` checks the live instance identity against its descriptor.
`down` preserves named volumes, Garage keys, and the instance UUID; it removes the readiness descriptor.
Never delete volumes or regenerate missing keys as a routine restart procedure.

The default project is `screenroom-local`; `SCREENROOM_BACKEND_PROJECT` accepts an explicit `screenroom-<name>` project.
`SCREENROOM_BACKEND_PROFILE` is retired and fails validation.
`SCREENROOM_BACKEND_PORT` defaults to `50051`. The public edge binds `127.0.0.1` on that port.
PostgreSQL 18 with pgvector, Garage, catalog, user, worker, playback gateway, and Envoy share this named instance.
Private database, object-store, and internal RPC ports have no host publication.
The optional index operator has a separate loopback HTTP port.

Runtime files live under `build/backend-runtime/<project>/`. Preserve `runtime.env` and `identity.json` with the associated volumes.
The helper refuses implicit identity or key rotation when volumes already exist.
`descriptor.json` contains the secret-free instance ID, public origin, client network, local user selector, and native/browser readiness flags.
The instance ID is not an authentication token.
A new installation creates one random UUID named `Local user` and persists its selector in `local-user.json`.
Create-once initialization preserves current users and links; it does not seed demo identities, catalog fixtures, or provider content.

Standalone client suites attach to this instance; they never start an alternate fixture API.
The root E2E runner owns startup and teardown unless `SCREENROOM_BACKEND_ATTACH=1` explicitly selects attachment.
Both endpoint aliases must match the descriptor origin: `SCREENROOM_API_ENDPOINT` and `SCREENROOM_E2E_API_ENDPOINT`.
Linux joins the descriptor's client network and forwards bytes from loopback to `edge:50051`.
Android uses same-port ADB reverse. Web uses ports `4173` and `4176` for development and production checks.

### Local index operator

The [index maintenance guide](index-maintenance.md) owns canonical poster repair, security, and Web opt-in.
The [canonical API](indexing.md#canonical-operator-api) adds backend policy, queue diagnostics, progress, attempts, and terminal redrive routes.
The operator supports managed connection discovery/save and canonical indexing; connection operations contact Jellyfin and write private credential files.
The [Jellyfin guide](jellyfin.md) defines those operations, private ownership, and unsupported edit/rotation paths.
The Web Maintenance screen supports managed setup through shared operator methods and guarded proxy routes.
The listener serves no frontend files and requires no frontend build.
The operator starts no worker and needs no embedding credentials or private object-store keys.
Older maintenance HTTP routes and browser operation-journal recovery have no supported compatibility interface.

The [lifecycle helper](../scripts/backend-lifecycle.py) requires an owned runtime with `runtime.env`, `identity.json`, `descriptor.json`, and `lifecycle.lock`.
The descriptor must mark the matching instance ready.
Startup also requires matching public instance identity, healthy owned PostgreSQL, and the restricted `operator_runtime` login role.
The helper does not initialize these prerequisites.

Prepare the backend image before launch, from the repository root:

```sh
docker build -f docker/backend/Dockerfile -t screenroom-backend:local .
```

The [backend Dockerfile](../docker/backend/Dockerfile) includes `screenroom-index-operator`.
This build command does not start services.
Launch requires explicit approval: operator startup applies forward-only database migrations before it opens the listener.
With an approved ready local instance, use:

```sh
bash scripts/backend.sh operator-up
bash scripts/backend.sh operator-status
bash scripts/backend.sh operator-down
```

Preserve the instance's explicit `SCREENROOM_BACKEND_PROJECT` and `SCREENROOM_BACKEND_PORT` values for each command.
The default operator origin is `http://127.0.0.1:50072`.
`SCREENROOM_OPERATOR_PORT` selects an explicit nonconflicting port.
If set, `SCREENROOM_OPERATOR_ORIGIN` must equal `http://127.0.0.1:<operator-port>` exactly.
The helper rejects reserved client/public ports, including the retired profile port `50073`.

`operator-up` uses `up -d --no-deps --no-build --pull never index-operator` with the base Compose file.
It starts only the operator and waits for its database-backed health endpoint.
`operator-status` checks public instance identity and operator health.
`operator-down` stops only `index-operator`; it preserves the backend, worker, volumes, keys, and descriptor.
Worker activation and provider work require separate explicit approval.
These lifecycle commands do not opt the Web app into operator access or replay historical browser records.
Web opt-in requires a separate restart with the [explicit attachment configuration](index-maintenance.md#explicit-local-attachment).
Local jobs, worker execution, runtime identity, and provider activation retain their current boundaries.

### Database startup and migrations

Every database-backed service and the local user initializer construct `ServiceDatabaseConfig` and call `db::connect_service` before listeners, work, or readiness.
The shared [database module](../crates/screenroom-backend/src/db.rs) applies ordered root [migrations](../migrations/) through SQLx's advisory lock.
A short-lived DDL pool closes before the restricted runtime pool opens. Migration or runtime-connection failure prevents startup.
`DATABASE_URL` selects runtime credentials. `SCREENROOM_MIGRATION_DATABASE_URL` optionally selects separate credentials for the same database target.
Without that variable, startup uses `DATABASE_URL` for migrations too.
Separate credentials restrict runtime queries, not process privileges: the process still holds the migration credential and DDL capability.
The runtime NOLOGIN roles define restricted user, catalog, worker, operator, and playback grants.
Compose uses the separate `operator_runtime` login through `screenroom_operator`.
The `screenroom-migrate` executable supports optional manual migration; no mandatory one-shot migration service precedes normal startup.
Add forward-only migrations. Never edit applied migration files or add down/reset procedures.
Applied migrations `001` through `019` remain immutable.
Forward-only migrations `010` through `019` implement the shared queue, canonical policies, settlement, source admission, receipt resolutions, and deadline eligibility.
Migration `010` moves jobs into `queue.jobs` and preserves job identities; it supplies no legacy `ingest.jobs` view.
Migration `020` drops only the exclusive obsolete `ingest.maintenance_starts` table, without `CASCADE`.
Migrations `021` and `022` add managed connections and restricted private read grants.
Migration [`202611140023_playback.sql`](../migrations/202611140023_playback.sql) adds playback sessions, receipts, cleanup, private resource metadata, freshness fences, and restricted grants.
The cutover preserves catalog, artwork, embeddings, users, links, installation identity, and canonical receipt provenance.
Migration source does not establish the applied schema of any persistent instance.
It supplies no legacy queue compatibility view or permission to reset persistent state.
Use runtime `sqlx::query`, `query_as`, and `query_scalar`; avoid compiled query macros and offline query metadata.
`migrate!` and `FromRow` derive remain supported.

### Durable ingest and identity

The [shared queue](indexing.md#queue-and-domain-responsibilities) provides transport for every ingest, embedding, artwork, and maintenance path.
The [canonical policy contract](indexing.md#policy-model) defines effective modes, selection strategies, retry defaults, budgets, and unsupported combinations.

The catalog has installation scope, not user ownership or access grants.
It has no user-owned search documents, backend preferences, external identity system, or transactional outbox.
Private managed provider connections remain separate from public catalog models.
Identity stores local users and validated links between stable source keys and Jellyfin user IDs.
Public profile selection validates enabled sources and eligible profiles. No private unchecked SetLink/ClearLink service or initializer link import exists.
Private `screenroom.user.internal.v1.PlaybackStateService.GetContinueWatching` reads mapped provider state; catalog hydration applies the requested source/library scope.
Absent mappings return empty progress. Personal progress never enters search documents or vectors.

PostgreSQL transactions couple domain progress, checkpoints, page data, and follow-up enqueue with shared-queue lease and fence checks.
Each source permits one active ingest run; canonical source concurrency accepts only one.
A run retains immutable policy and selection snapshots beside the current policy.
Runs that require embeddings pin a model identity; follow-ups also bind input hashes and compatible worker capabilities.
This design supplies no policy-history service, broker, or public index API.
Live Jellyfin full, reconcile, and incremental traversal remains non-authoritative and does not reconcile absence through deletion or membership removal.
Generic ingest completion grants no absence authority and does not automatically disable the policy.
Partial results and budget stops never authorize deletion.
Mutable execution accepts only validated per-work budgets and canonical admission.
Historical lifetime checkpoint decoding remains read-only; it does not restore obsolete execution or mutation APIs.
Invalid checkpoint data fails with safe errors.
Resume counters distinguish a paused segment from cumulative totals.
Canonical lifetime request reservations and record observations never reset on resume or redrive.
Record budgets include unchanged touches; `run.items` counts only metadata writes.
Deadline expiry fails the run and cancels active jobs; expired work cannot redrive.
Terminal redrive retains attempts, receipts, coverage provenance, and consumed budgets; it differs from paused-run resume and ambiguous-start replay.

### Private source provisioning

The [managed connection API](jellyfin.md#connection-api) provisions private connection references and canonical source/profile bindings.
User and worker resolve current connections from PostgreSQL without manual provider JSON, enrollment commands, or service restart.
No manual catalog JSON initializer, file-backed `ProviderRegistry`, or private catalog `IngestionService` forms part of source provisioning.
The canonical operator HTTP API owns admission and run controls; internal progress reads remain separate.
The [binding contract](indexing.md#source-provisioning-and-locator-binding) retains source snapshots, capability checks, and fail-closed admission.
The worker CLI accepts only `serve`, `drain`, and `activate-model`; old fixture, sample, index, and run-control commands are not interfaces.
Canonical configure/start and run controls use the operator API.
Provider acquisition and worker activation require explicit approval for the exact target.

`SCREENROOM_JELLYFIN_CREDENTIAL_DIRECTORY` selects `/run/screenroom/credentials` in Compose.
The persistent credential volume requires directory mode `0700`, generated UUID files mode `0600`, and effective UID `10001`.
Operator mounts it read/write; user, worker, and playback gateway mount it read-only; catalog has no mount.
Private database references never expose keys or locators to ordinary clients. This is not encryption at rest.
The [credential contract](jellyfin.md#credential-ownership) defines no-follow file checks, ownership, synchronization, and recovery limits.

### Playback gateway runtime

The [playback guide](playback.md) owns public behavior, capabilities, personal identity, and media limits.
The gateway owns cleanup supervision; playback never enters the indexing queue.
The [runtime](../crates/screenroom-playback-gateway/src/runtime.rs) validates private directories and applies migrations before listener startup.
It requires a restricted playback role and an exclusive PostgreSQL advisory lock.
It revokes abandoned transports before readiness; restart resumes cleanup, not playback or personal-report replay.
Loss of database ownership closes active dispatch.

| Configuration | Contract |
| --- | --- |
| `SCREENROOM_PLAYBACK_GATEWAY_LISTEN` | Socket address; standalone default `127.0.0.1:50054`, Compose private listener `0.0.0.0:50054`. |
| `SCREENROOM_PLAYBACK_API_ORIGIN` | Required client-visible HTTP(S) API origin for media URLs; Compose uses the shared loopback edge. |
| `SCREENROOM_PLAYBACK_GATEWAY_CORS_ORIGINS` | Exact comma-separated SPA origins; empty permits no cross-origin browser access. |
| `SCREENROOM_JELLYFIN_CREDENTIAL_DIRECTORY` | Required managed administrator-key directory; read-only gateway mount. |
| `SCREENROOM_PLAYBACK_SECRET_DIRECTORY` | Required separate private playback store; gateway-only read/write mount. |
| `DATABASE_URL` | Restricted runtime login; Compose uses `playback_runtime` with `screenroom_playback` grants. |
| `SCREENROOM_MIGRATION_DATABASE_URL` | Separate startup DDL credential for the same database instance. |

The executable rejects database owners, superusers, schema-create privileges, and writes to tables outside the playback schema.
The process still holds migration credentials; runtime query restrictions do not establish process-level DDL isolation.
The private `/ready` endpoint has no public Envoy route.
Readiness establishes startup prerequisites, not provider availability, media decode, or successful remote cleanup.

Compose mounts `backend-playback-secrets` at `/run/screenroom/playback`.
The directory requires UID/GID `10001:10001` and mode `0700`; private UUID files require mode `0600`.
Managed keys and playback secrets require separate, non-nested directories.
Only the gateway receives the playback volume; PostgreSQL stores private references rather than tokens or replay capability material.
The [private store](../crates/screenroom-backend/src/playback/secrets.rs) checks ownership, modes, links, file type, and size.
It uses no-follow access, no-overwrite publication, and filesystem synchronization.
These files provide no encryption at rest.

The supported lifecycle `up` reapplies the additive [role bootstrap](../docker/backend/postgres.sql) before service startup.
Bootstrap creates only absent roles/logins; it never rotates current passwords or repairs arbitrary role drift.
PostgreSQL does not replay initdb scripts on a current data volume.
Direct Compose startup alone therefore cannot ensure roles on an older volume.
The lifecycle helper also initializes a new empty playback volume from root-owned `0755` to `10001:10001:0700`.
It refuses arbitrary ownership repair for populated or mismatched private stores.
Direct Compose startup does not perform that volume initialization.
Preserve both private volumes with their database and installation identity.
Do not delete volumes, reset passwords, or replace private files as a restart workaround.

Activation requires separate approval for the exact installation, migrations, role bootstrap, private-volume initialization, and service lifecycle.
Provider acceptance additionally requires the exact server, personal profile, titles, and provider-write effects.
Source configuration does not establish actual grants, mount permissions, container behavior, or live provider compatibility.

#### Cleanup and shutdown

Stop revokes local media before remote cleanup and reports the final known position under the ordered event contract.
Transport replacement queues old-encoding cleanup without a false item stop or another item start.
Durable intent records exact gateway-owned device and play-session identities before remote effects.
Lost authorization or token responses use exact-device recovery with the managed administrator key.
Cleanup verifies server identity; it never uses profile-wide device filters or another session's token.
The gateway cleans old encodings before final device revocation.
An uncertain personal effect keeps watched freshness unknown even after credential revocation or successful cleanup.

Cleanup uses durable bounded claims and at most five attempts; exhausted or identity-change failures remain visible through safe diagnostics.
Retention preserves incomplete cleanup evidence and deletes private files only after durable retirement proof.
Shutdown closes control admission, drains owned tasks, and classifies durable dispatch outcomes before cleanup.
If classification fails, shutdown defers cleanup rather than assume that cancelled provider work stopped.
The shutdown budget is 61 seconds; Compose and lifecycle teardown allow 75 seconds.
The lifecycle helper uses `down --timeout 75` and preserves named volumes.

#### Edge media policy

The [Envoy configuration](../docker/backend/envoy.yaml) gives playback RPC routes a 35-second timeout.
Media GET/HEAD/OPTIONS routes use no whole-request timeout and a 60-second idle timeout.
The HTTP connection manager bounds request headers to 10 seconds and 32 KiB.
The adapter separately bounds provider headers and idle waits to 10 seconds.
Access logs use query-free `%PATH(NQ:PATH)%`, not capability-bearing request URLs.
Media routes disable all trace sampling and remove client trace-forcing/context headers.
The gateway adds `no-store`, `no-referrer`, and `nosniff` response headers.
The Web document also declares no-referrer; anonymous cross-origin video/HLS/subtitle requests omit cookies.
These controls do not make bearer URLs safe to export or log elsewhere.

### Watched snapshots

The provider-neutral [user-state adapter contract](../crates/screenroom-catalog-adapter/src/user_state.rs) defines `UserStateAdapter`, `UserStateScope`, and complete `WatchedSnapshot` generations.
Watched observations belong to personal state, not catalog metadata, search documents, vectors, or resume progress.
The user service alone bridges current Jellyfin identity links into this generic acquisition contract.
The catalog overlay reads PostgreSQL in batches for the selected local user and chosen source-item rows; it never merges arbitrary provider matches.
The separate Continue Watching path can still request provider state.

The [watched repository](../crates/screenroom-backend/src/watched.rs) atomically replaces each complete user/source generation and its observations.
It retains acquisition `fetched_at` and the local user's `updated_at` revision as a publication fence.
Link changes invalidate that revision, even after a remap back to the original profile.
Older acquisitions cannot replace newer generations for the same revision.
Snapshots expire 900 seconds after acquisition starts, not after a catalog read or failed refresh.
Failures preserve the previous generation and timestamp until expiry; absent or expired observations mean unknown, never unwatched.
Unsupported adapters and absent or disabled profiles invalidate the applicable generation.
Disabled local users or sources cannot expose observations.

The [user service scheduler](../crates/screenroom-user/src/watched.rs) resolves managed connections and current personal links dynamically.
It uses backend-private credentials and checks actual server identity; clients receive neither credentials nor connection locators.
Its fixed bounds are:

| Bound | Value |
| --- | --- |
| First sweep | Immediately after startup. |
| Sweep interval | 300 seconds; missed ticks do not accumulate. |
| Concurrent acquisitions | Two. |
| Bindings per sweep | At most 64, with a rotating keyset cursor. |
| Sweep wall-clock budget | 240 seconds. |
| Acquisition timeout per binding | 180 seconds. |

The [Jellyfin adapter](../crates/screenroom-catalog-adapter/src/jellyfin/watched.rs) requests explicit `Played` values for movies and episodes across selected enabled libraries.
It leaves series and seasons unknown and never derives watched state from resume position or aggregate container state.
Acquisition requires two identical complete passes and an enabled-profile check before and after both passes.
Duplicate IDs, partial pages, total-count changes, and differences between passes reject the generation.
These checks detect observed churn; they do not guarantee an immutable provider snapshot.
Bounds permit 32 libraries, 50,000 observations total, and a 50,000-item limit per library.
Both passes share a 1,200-page request budget, with at most 100 items per page.
Timeouts and capacity failures retain the previous timestamp; they do not publish partial generations.

Forward-only [migration `202611110009_watched_state.sql`](../migrations/202611110009_watched_state.sql) adds the personal snapshot tables and restricted catalog/user grants.
Migrations `202611100001` through `202611100008` remain immutable.
Service startup applies migrations through the shared startup path described above.
Code and documentation changes do not authorize migration application, service restart, provider acquisition, or live activation.
Those operations require separate explicit approval for the target instance and provider.
The watched overlay adds no manual watched controls.
Playback reports do write personal provider state under the [playback freshness contract](playback.md#jellyfin-identity-and-personal-state).
Acquisition and publication share a separate playback epoch; pending or unknown effects suppress stale watched observations.
The [client watched contract](client-api.md#watched-state) owns public values, web presentation, and read freshness.

### Search

PostgreSQL full-text search and `pg_trgm` title similarity supply lexical results.
Exact pgvector cosine retrieval covers all eligible source documents with 1024-dimensional vectors, not a lexical shortlist.
Coverage and vector candidates share one SQL statement snapshot and the same current-version/hash predicate.
No HNSW, IVFFlat, approximate nearest-neighbor index, or reranker participates.
Fixed reciprocal-rank fusion uses `k=60` on source-document ranks, then retains each canonical title's best document.
Multiple sources do not produce an additive canonical-title boost. Display-metadata precedence remains separate from relevance.
One active model revision serves queries; an explicit rebuild switch requires complete current coverage before activation.

Worker and query paths use the same whitespace preprocessing and hash version.
Configure `SCREENROOM_EMBEDDING_BASE_URL`, `SCREENROOM_EMBEDDING_MODEL`, `SCREENROOM_EMBEDDING_DIMENSIONS=1024`, and `SCREENROOM_EMBEDDING_REVISION`.
For example, use `https://embedding.example.test/v1` with an operator-managed `local-embedding` model alias.
`SCREENROOM_EMBEDDING_API_KEY` supplies an optional explicit secret. Clients never receive these settings.
The revision is an operator declaration, not proof of an immutable upstream model revision.
Unavailable, incompatible, or incomplete semantic coverage returns explicit lexical degradation when a lexical method remains active.
`effective_methods.semantic` becomes false and `degradation_reason` explains the fallback; semantic-only requests fail instead of fabricated success.
See the [client search contract](client-api.md#search-limits) for query, filter, and pagination rules.

### Artwork storage and publication

Garage stores private path-style S3 objects. Catalog uses a read key; worker uses a separate read/write key.
`storage::object_key` uses `artwork/v1/sources/{source_uuid}/sha256/{first_two_hex}/{sha256}/original`.
Clients receive no S3 credentials, presigned URLs, redirects, or direct provider image URLs.
The worker validates original/native bytes and SHA manifests. Native artwork is mandatory for publication.

Catalog item hydration resolves episode cards to the season poster, then the series poster. Both ancestors use the target's selected source and eligible source/provider/library scope. Kind filters select targets, not ancestors. Only complete published artwork of kind `poster` qualifies; episode artwork is never a fallback. Without an eligible ancestor poster, the item has no artwork. Season `ListChildren` keeps own artwork for actual episode children; other item kinds retain their own primary-artwork ranking. The resolver uses read-only batches, with no metadata changes, reindex, or provider requests.
An atomic page commit allocates a monotonic revision and enqueues variant work.
The publication pointer changes only after the full required set passes upload verification and reaches database-ready state.
The previous complete image remains visible while its replacement lacks required variants; published revision URLs remain immutable.
Optional absent or invalid artwork produces diagnostics, not metadata failure; the next ingest retries repair.
No fabricated PNG, automatic physical object deletion, or garbage-collection policy forms part of this implementation.

Catalog exposes GET/HEAD `/artwork/{uuid}/revisions/{positive_revision}/{native|original}` on the public origin.
It caps public images at 4 MiB and streams with backpressure; client disconnect drops the upstream response.
Responses retain exact MIME, length, SHA-based ETag, `nosniff`, and `private, max-age=0, must-revalidate`.
Conditional requests use database publication availability before 304. Unknown or disabled artwork returns 404; storage failures return 503.
Artwork visibility follows the shared installation, not a per-user ACL.

### Direct listeners

The service `src/main.rs` files own environment defaults; Compose supplies private container addresses.
Standalone defaults are catalog `127.0.0.1:50052`, user `127.0.0.1:50053`, and playback gateway `127.0.0.1:50054`.
User's internal default is `127.0.0.1:50063`. Catalog exposes no private ingestion RPC listener.

Each listen variable accepts a socket address, not an HTTP URL.
Startup emits a JSON log with the service name and actual listener address.
Stop a service with Ctrl-C; Unix services also accept SIGTERM.
The Tonic server drains before the retained trace provider shuts down. Listener or server errors also trigger provider shutdown.

Each crate's `src/lib.rs` and transport modules own server construction and handlers.

### Browser CORS

| Environment variable | Default |
| --- | --- |
| `SCREENROOM_CATALOG_PUBLIC_ORIGIN` | Required; no default. Use the client-visible catalog origin. |
| `SCREENROOM_CATALOG_CORS_ORIGINS` | Empty; no allowed browser origins. |
| `SCREENROOM_USER_CORS_ORIGINS` | Empty; no allowed browser origins. |
| `SCREENROOM_PLAYBACK_GATEWAY_CORS_ORIGINS` | Empty; no allowed browser origins. |

Each CORS variable accepts comma-separated absolute HTTP(S) origins, such as `http://127.0.0.1:4173,http://localhost:4173`.
The parser trims whitespace and ignores empty entries. It rejects credentials, non-root paths, queries, and fragments.
Use the exact browser origin without a trailing slash; the allowlist compares origin values exactly.
Without an explicit matching origin, cross-origin browser gRPC-Web access fails.
With approved database configuration present, these direct-listener commands permit the browser origin `http://127.0.0.1:4173`.
The catalog example declares its default direct origin, `http://127.0.0.1:50052`; clients must use that same endpoint.
Database configuration alone is insufficient: catalog startup requires `SCREENROOM_CATALOG_PUBLIC_ORIGIN` after startup migrations, before listeners open.
The database, search, and artwork sections above describe the other operational runtime configuration.

```sh
SCREENROOM_CATALOG_PUBLIC_ORIGIN=http://127.0.0.1:50052 \
  SCREENROOM_CATALOG_CORS_ORIGINS=http://127.0.0.1:4173 \
  cargo run --locked -p screenroom-catalog --bin screenroom-catalog
SCREENROOM_USER_CORS_ORIGINS=http://127.0.0.1:4173 cargo run --locked -p screenroom-user
```

Direct gateway startup also requires every [playback runtime prerequisite](#playback-gateway-runtime), not CORS alone.
Catalog permits POST, GET, and HEAD with browser preflight; its allowlist includes user-selector and conditional-artwork headers.
Catalog exposes `grpc-status`, `grpc-message`, `etag`, and `content-length`.
Consult each transport module for its exact header allowlist.
CORS is not authentication and does not restrict native clients. This configuration grants no permission for production exposure.
Playback control and media use distinct header/method allowlists from the [playback contract](playback.md#media-and-lifetime-rules).
Compose permits localhost and `127.0.0.1` SPA origins on ports `4173`, `4176`, and `5173`.

## Shared telemetry

The `telemetry` feature in [screenroom-observability](../crates/screenroom-observability/src/telemetry.rs) provides:

- `TelemetryConfig::from_env()` for environment configuration.
- `Telemetry::init(service_name, config)` for JSON logs, trace and meter providers, and W3C Trace Context propagation.
- `Telemetry::meter()` for bounded queue and worker instruments; `force_flush()` and `shutdown()` cover both providers.
- `rpc_layer()` for the community `tonic-tracing-opentelemetry` server middleware, re-exported as `OtelGrpcLayer`.
- `shutdown_signal()` for process shutdown signals and `Telemetry::shutdown()` for provider shutdown.

Each executable supplies its package name as the OpenTelemetry `service.name` resource attribute.
The shared crate configures SDK/exporter setup; it does not define a custom RPC telemetry framework.
The workspace selects community `tonic-tracing-opentelemetry` 0.42.1 with `tracing_level_info`, Tonic 0.14, OpenTelemetry 0.33.1, and tracing-opentelemetry 0.34.
The middleware is not an official OpenTelemetry component; the SDK and OTLP exporter use official OpenTelemetry crates.

| Environment variable | Behavior |
| --- | --- |
| `RUST_LOG` | Tracing filter; defaults to `info`. |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | Explicit HTTP(S) traces endpoint, including its path; takes precedence over the general endpoint. |
| `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` | Explicit HTTP(S) metrics endpoint, including its path; takes precedence over the general endpoint. |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | HTTP(S) collector base endpoint; setup appends `/v1/traces` and `/v1/metrics` for each signal without an explicit override. |

Even with `RUST_LOG=trace`, shared setup suppresses DEBUG and TRACE output for targets with these prefixes:
`opentelemetry-otlp`, `opentelemetry_otlp`, `opentelemetry-http`, `opentelemetry_http`, `reqwest`, `hyper`, and `h2`.
This INFO-level cap limits transport diagnostics that can expose endpoint paths, query tokens, headers, or response bodies.
The cap does not assert that all other logs exclude sensitive data.
Setup also excludes unsafe `HttpTraceClient.PartialSuccess` and `HttpMetricsClient.PartialSuccess` collector-response events on exact `opentelemetry-otlp`/`opentelemetry_otlp` targets.
This locked-dependency protection does not establish general redaction.

With all endpoint variables unset, startup needs no collector and exports neither OTLP traces nor OTLP metrics.
An explicit endpoint enables the corresponding official OTLP HTTP/protobuf exporter, not OTLP gRPC.
Metrics use a periodic exporter; setup does not export logs through OTLP.
The [metric definitions](../crates/screenroom-observability/src/metrics.rs) accept bounded enum labels, not UUIDs, arbitrary source keys, locators, or raw errors.
Queue counts represent observations, not worker health; reserved-request counters do not represent exact wire traffic.
The worker attaches per-operation transport observers to provider JSON, capabilities, delta, artwork, embeddings, and storage PUT/HEAD/verification GET.
Byte counters report consumed body chunks by request kind and direction, including partial failure and cancellation.
They do not measure wire bytes or prove server acknowledgement.
Transport HTTP success does not establish decode, indexing, or publication success.
The [transport contract](indexing.md#progress-completion-and-metrics) distinguishes resource observations from durable progress and budgets.
Exporter configuration does not establish collector delivery or an exporter-to-collector verification guarantee.
The observability crate retains no default features; telemetry exporters remain optional dependencies.
With approved database configuration present, this example uses a local collector that accepts OTLP HTTP/protobuf.
It also declares the required catalog origin for clients that use the default direct listener:

```sh
OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=http://127.0.0.1:4318/v1/traces \
  SCREENROOM_CATALOG_PUBLIC_ORIGIN=http://127.0.0.1:50052 \
  RUST_LOG=info cargo run --locked -p screenroom-catalog --bin screenroom-catalog
```

The middleware creates RPC spans, extracts remote trace context, and records basic RPC fields and response-header status.
It does not inspect final gRPC trailers or cover the full response-stream lifetime.
Do not treat its status fields as complete final gRPC outcomes.
The middleware records User-Agent and Host values; it does not capture arbitrary headers or message bodies.
Those fields still require a privacy review before production use; this setup asserts no universal privacy guarantee.

## Backend verification

The [testing guide](testing.md) separates isolated fault tests, disposable PostgreSQL checks, and real-provider acceptance.
No broad test command grants database, provider, Docker lifecycle, or GUI authority.
The [backend contract helper](../scripts/backend-contract.py) checks an approved live local instance through real RPC and artwork paths.
Its use does not authorize backend startup, teardown, provider imports, or volume removal.

## Protobuf generation

The [Buf configuration](../buf.gen.yaml) generates Prost types and Tonic service code into `crates/screenroom-proto/src/generated/`.
Install `protoc-gen-prost` and `protoc-gen-tonic` on `PATH` before generation.
From the repository root:

```sh
buf lint
buf generate
```

Catalog owns BrowseService, CatalogService, SearchService, and catalog messages; user owns UserService and local user messages.
Common owns the shared SearchTypes and SearchMethods records.
The ordinary client exposes public catalog/common/user/playback RPC contracts, not private operator/internal RPC bindings or backend database dependencies.
The separate [operator capability](client-api.md#operator-capability) uses typed HTTP/JSON through explicit `OperatorClient` construction.
The contract supplies no old PreferencesService or mock-profile aliases.
