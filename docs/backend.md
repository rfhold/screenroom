# Backend development

The catalog, user, and index worker use shared PostgreSQL repositories and typed offline provider adapters.
The [component outline](components.md) owns intended product scope and the public deployment contract.
The [client API guide](client-api.md) owns the public client contract and local settings boundary.

## Transport boundary

| Executable | Registered services | Business behavior |
| --- | --- | --- |
| `screenroom-catalog` | `screenroom.catalog.v1.BrowseService`, `screenroom.catalog.v1.CatalogService`, and `screenroom.catalog.v1.SearchService` | Persistent scoped catalog, search, artwork credits, and artwork HTTP transport. |
| `screenroom-user` | `screenroom.user.v1.UserService` | GetCurrentUser selects an enabled local user through UUID metadata. |
| `screenroom-playback-gateway` | None | Empty Tonic routes; the `screenroom.playback_gateway.v1` protobuf package defines no messages or RPCs. |

Each server accepts HTTP/2 gRPC and HTTP/1 gRPC-Web through `tonic-web` on one listener.
Envoy routes public catalog and user paths through one loopback edge. It does not expose operator or internal playback-state RPCs.
The playback gateway retains empty routes. No service implements authentication, JWT verification, or playback.

Local user UUIDs select state; they are not credentials. Private listeners provide network separation, not authentication or administrator validation.
Do not expose the local stack to an untrusted network.

## Persistent local instance

Backend startup requires explicit approval, Docker Compose, Python 3, grpcurl, and the configured embedding endpoint.
Startup builds the backend image unless `SCREENROOM_BACKEND_BUILD=0` selects a current local image.
From the repository root:

```sh
bash scripts/backend.sh up
bash scripts/backend.sh wait
bash scripts/backend.sh status
bash scripts/backend.sh down
```

These commands mutate local runtime state; they are not prose-validation commands.
`up` initializes missing installation files, private Garage keys, and local users, then starts the persistent instance.
Its create-only user initialization preserves operator names, disabled users, cleared links, and remapped links.
The explicit manual user-init mode also supports updates; service startup itself does not create users.
`wait` checks real catalog/user RPCs, document coverage, semantic search, artwork, and public/private path boundaries.
HTTP 200 alone does not establish readiness. `status` checks the live instance identity against its descriptor.
`down` preserves named volumes, Garage keys, and the instance UUID; it removes the readiness descriptor.
Never delete volumes or regenerate missing keys as a routine restart procedure.

The default project is `screenroom-e2e`; `SCREENROOM_BACKEND_PROJECT` also accepts `screenroom-e2e-<suffix>`.
`SCREENROOM_BACKEND_PORT` defaults to `50051`. The only host listener binds `127.0.0.1` on that port.
PostgreSQL 18 with pgvector, Garage, catalog, user, worker, and Envoy share this named instance.
Private database, object-store, operator, and internal RPC ports have no host publication.

Runtime files live under `build/backend-runtime/<project>/`. Preserve `runtime.env` and `identity.json` with the associated volumes.
The helper refuses implicit identity or key rotation when volumes already exist.
`descriptor.json` contains the secret-free instance ID, public origin, client network, local user, model declaration, and readiness evidence.
The instance ID is not an authentication token.
Default bootstrap users are Alice (`00000000-0000-0000-0000-000000000001`) and Bob (`00000000-0000-0000-0000-000000000002`).

Standalone client suites attach to this instance; they never start an alternate fixture API.
The root E2E runner owns startup and teardown unless `SCREENROOM_BACKEND_ATTACH=1` explicitly selects attachment.
Both endpoint aliases must match the descriptor origin: `SCREENROOM_API_ENDPOINT` and `SCREENROOM_E2E_API_ENDPOINT`.
Linux joins the descriptor's client network and forwards bytes from loopback to `edge:50051`.
Android uses same-port ADB reverse. Web uses ports `4173` and `4176` for development and production checks.

### Database startup and migrations

Every database-backed service and catalog/user initialization command constructs `ServiceDatabaseConfig` and calls `db::connect_service` before listeners, work, or readiness.
The shared [database module](../crates/screenroom-backend/src/db.rs) applies ordered root [migrations](../migrations/) through SQLx's advisory lock.
A short-lived DDL pool closes before the restricted runtime pool opens. Migration or runtime-connection failure prevents startup.
`DATABASE_URL` selects runtime credentials. `SCREENROOM_MIGRATION_DATABASE_URL` optionally selects separate credentials for the same database target.
Without that variable, startup uses `DATABASE_URL` for migrations too.
Separate credentials restrict runtime queries, not process privileges: the process still holds the migration credential and DDL capability.
The runtime NOLOGIN roles define identity-only user, catalog, and worker grants.
The `screenroom-migrate` executable supports optional manual migration; no mandatory one-shot migration service precedes normal startup.
Add forward-only migrations. Never edit applied migration files or add down/reset procedures.
Use runtime `sqlx::query`, `query_as`, and `query_scalar`; avoid compiled query macros and offline query metadata.
`migrate!` and `FromRow` derive remain supported.

### Durable ingest and identity

The catalog has installation scope, not user ownership or access grants.
It has no user-owned search documents, backend preferences, provider-connection records, external identity system, or transactional outbox.
Identity stores local users and operator-controlled links between stable source keys and Jellyfin user IDs.
Private `screenroom.user.internal.v1.PlaybackStateService.GetContinueWatching` reads mapped typed mock state; catalog hydration applies the requested source/library scope.
Absent mappings return empty progress. Personal progress never enters search documents or vectors.

PostgreSQL jobs retain leases, fences, checkpoints, page data, and follow-up work atomically.
Each source permits one active ingest run. A run retains immutable policy and selection snapshots beside the current policy.
This design supplies no policy-history service, broker, or public index API.
Jellyfin removal reconciliation requires a complete, untainted traversal of selected libraries.
Partial results and budget stops do not authorize deletion. Invalid checkpoint data fails with safe errors.
Resume counters distinguish a paused segment from cumulative totals.

TMDB supports seven configurable bounded feeds: movie popular, upcoming, now playing, and trending; TV popular, on the air, and trending.
Feed membership and ordinal position remain separate from title existence.
A durable typed frontier expands series into seasons and episodes; linked IDs support enrichment.
The adapters do not crawl live providers or promise complete external catalogs.

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
Standalone defaults are catalog `127.0.0.1:50052`, user `127.0.0.1:50053`, and empty playback gateway `127.0.0.1:50054`.
Catalog's private operator default is `127.0.0.1:50062`; user's internal default is `127.0.0.1:50063`.

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
SCREENROOM_PLAYBACK_GATEWAY_CORS_ORIGINS=http://127.0.0.1:4173 cargo run --locked -p screenroom-playback-gateway
```

Catalog permits POST, GET, and HEAD with browser preflight; its allowlist includes user-selector and conditional-artwork headers.
Catalog exposes `grpc-status`, `grpc-message`, `etag`, and `content-length`.
Consult each transport module for its exact header allowlist.
CORS is not authentication and does not restrict native clients. This configuration grants no permission for production exposure.
Playback gateway routes remain empty.

## Shared telemetry

The `telemetry` feature in [screenroom-observability](../crates/screenroom-observability/src/telemetry.rs) provides:

- `TelemetryConfig::from_env()` for environment configuration.
- `Telemetry::init(service_name, config)` for JSON logs, the trace provider, and W3C Trace Context propagation.
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
| `OTEL_EXPORTER_OTLP_ENDPOINT` | HTTP(S) collector base endpoint; setup appends `/v1/traces`. |

Even with `RUST_LOG=trace`, shared setup suppresses DEBUG and TRACE output for targets with these prefixes:
`opentelemetry-otlp`, `opentelemetry_otlp`, `opentelemetry-http`, `opentelemetry_http`, `reqwest`, `hyper`, and `h2`.
This INFO-level cap limits transport diagnostics that can expose endpoint paths, query tokens, headers, or response bodies.
The cap does not assert that all other logs exclude sensitive data.
Setup also excludes the unsafe `HttpTraceClient.PartialSuccess` collector-response event at all levels on exact `opentelemetry-otlp`/`opentelemetry_otlp` targets: locked-dependency protection, not general redaction.

With both endpoint variables unset, startup needs no collector and exports no OTLP traces.
An explicit endpoint enables the official OTLP HTTP/protobuf trace exporter, not OTLP gRPC.
This setup does not export logs or metrics through OTLP.
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

From the repository root:

```sh
cargo fmt --all -- --check
cargo test --workspace --all-features --locked
```

Database integration tests require explicit disposable PostgreSQL targets; inspect each test's environment and ignore gate before execution.
Never point destructive test setup at the persistent development database.
Ignored live embedding tests require separate approval for their exact endpoint; ordinary workspace tests do not authorize external requests.
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
Public client-core bindings contain only catalog/common/user contracts, not operator or internal RPC bindings or backend database dependencies.
The contract supplies no old PreferencesService or mock-profile aliases.
