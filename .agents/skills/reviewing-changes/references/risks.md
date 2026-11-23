# Screenroom Risk Inputs

Paths below start at the repository root.
Use the [root router](../../../../AGENTS.md) and nearest scoped instructions for each affected subtree.

## Queue, Worker, and Indexing

| Surface | Coupled paths | Required contract evidence |
| --- | --- | --- |
| Queue primitives | `crates/screenroom-queue/src/lib.rs`, `migrations/`, backend queue tests | Transactional enqueue, run-scoped dedupe, claims, leases, fences, attempts, classified retry, and durable redrive receipts. |
| Domain admission | `crates/screenroom-backend/src/{ingest,indexing,repair,queue_admin}.rs` | Enabled source/policy, immutable run snapshot, source cooldown, capabilities, deadlines, budgets, and guarded commit eligibility. |
| Worker effects | `crates/screenroom-index-worker/src/{full,live,pipeline,repair}.rs` | Stale claims cannot publish, advance checkpoints, or consume another run's dependencies. |
| Progress and retry | Queue and backend tests, worker durable tests | Preserve attempt history, logical resolutions, checkpoints, lifetime consumption, and separate resume, redrive, and start-request idempotency. |
| Policy/model identity | Backend indexing and worker paths | Pin immutable policy/source inputs; require model identity and input hashes only for embedding work. |
| Enumeration | Provider delta, backend ingest, worker full traversal | No absence deletion from partial traversal. Separate enumeration completion, settlement, and success. |
| Artwork | Backend media/storage, worker publication paths, client artwork paths | Publish only verified required variants. Preserve the prior complete generation and private object keys. |
| Diagnostics | Backend queue administration, operator DTOs, observability metrics | Separate raw state from claimability, blocked work, expired leases, terminal work, and historical failures. |

Keep the queue provider-neutral. Domain admission does not belong in provider transport.
Keep per-run publication dependencies independent; byte dedupe cannot orphan a dependent run.

## Providers and Private Data

- Apply the [canonical provider destination-security invariants](../../changing-providers/SKILL.md#destination-security) to provider transport and setup changes.
- Inspect `crates/screenroom-backend/src/connections.rs` and `crates/screenroom-backend/src/connections/secrets.rs` with worker and user-service consumers.
- Inspect adapter `crates/screenroom-catalog-adapter/src/jellyfin/{connection,secret,network,revision,delta,watched}.rs` for transport contracts.
- Managed database connections own runtime resolution; no file-backed provider registry or manual catalog initializer supplies runtime configuration.
- Runtime mocks, TMDB catalog fixtures, demo-user bootstrap, older maintenance APIs, and browser operation-journal replay are not supported product interfaces.
- Web setup uses managed private connections and shared operator RPCs; native apps supply no onboarding UI.
- Check URL/key discovery invalidation, exact in-memory UUID/body Retry, key cleanup, and no automatic mutation or browser-journal replay.
- Refresh requires a saved policy; no-policy Refresh never creates a configure-to-start chain.
- First-add supports no edit, key rotation, or connection/server replacement API or UI workflow.
- Keep keys, private locators, upstream bodies, and private profile bindings out of public records, errors, traces, and metrics.
- Source/profile UUIDs select data; neither selectors nor private network placement supply authentication.
- Check credential rotation against connection lifetime, source revision, worker admission, profile generations, and public redaction.
- Distinguish adapter locator digests from managed revisions that bind connection, credential reference, server identity, and normalized URL.
- Source admission also binds connection generation; runtime resolution separately verifies ServerId.
- Neither revision proves credential freshness or remote dataset identity; raw key contents remain outside both.
- Private files provide no encryption at rest; preserve strict owner/mode, no-follow access, no overwrite, and uncertain-commit retention.
- Local source and fault tests do not prove live TLS, container ownership, read-only mounts, or restricted-role deployment.
- Keep personal watched state separate from shared metadata.
- Jellyfin timestamp polls need overlap, durable windows/checkpoints, capability fallback, and periodic complete reconciliation.
- The inspected 10.10.7 filter defect lacks live reproduction; do not generalize it to every 10.10 release.

## Client Contracts and Platform Consumers

- Inspect `proto/`, `buf.gen.yaml`, generated Rust, shared client models, UniFFI/WASM exports, TypeScript models, and every application consumer.
- Generated protobuf files belong to Buf; ordinary domain records belong to `scripts/generate-client-models.py`.
- `Client.operator()` derives a distinct RPC capability; internal RPCs and SQLx do not cross the client model boundary.
- Preserve explicit scope, immutable client configuration, bounded caches, provenance, cancellation, and wide integer precision.
- Profile changes invalidate profile-bound caches and fence stale responses on all platforms.
- Inspect Swift cancellation patches, buffer ownership, future lifecycle, Kotlin coroutine ownership, and WASM abort behavior.
- Keep native UI state on platform UI threads or actors.
- Settings require explicit failures: Apple atomic JSON, Android validated commit, Linux private atomic XDG JSON, Web context-bound storage.
- Root Rust checks exclude the independent `apps/linux` workspace.
- Preserve native focus, empty/error states, pagination guards, per-item source identifiers, links, and ratings without mock-only UI assumptions.
- Reject global attribution footers and footer-only requests or state; preserve backend CatalogInfo, artwork credits, provenance, and rights APIs.
- Treat library IDs as opaque; literal `movies` or `tv` values imply no special behavior.
- Normalize old saved `tmdb` settings to `all` without a whole-settings reset; ignore obsolete subtitle fields in ordinary settings files.

## Operator and Shared Edge

- Inspect operator protobuf, shared client DTOs, Compose Envoy routes, and Pantheon Gateway routes together.
- Keep the operator on the trusted private edge with exact service prefixes and preserve internal RPC isolation.
- Preserve request/response bounds and lossless integer transport.
- The operator has no administrator authentication. CORS and user selectors do not grant authority.
- Keep the Web SPA fallback separate from static website prerender/no-client-JavaScript behavior.
- Preserve `website/static/404.html` in output to prevent Cloudflare Pages' implicit SPA fallback.

## Persistence, Search, and Observability

- Inspect `crates/screenroom-backend/src/db.rs` with every database service entry point.
- Preserve startup migrations before listeners, claims, and readiness; retain advisory locks and short-lived DDL pools.
- Applied migrations remain immutable and forward-only; preserve identities, catalog, artwork, embeddings, users, links, and receipt provenance.
- Runtime SQLx uses typed parameters and explicit row types; avoid compile-time database requirements and unchecked SQL interpolation.
- Search requires exact 1024-dimensional retrieval over the eligible document set and single-snapshot coverage.
- Preserve source-document RRF before canonical dedupe, explicit lexical degradation, query guards, and scope-bound pages.
- Do not substitute ANN, lexical shortlists, rerankers, or fake semantic success.
- Inspect `crates/screenroom-observability/{Cargo.toml,src/telemetry.rs,src/metrics.rs,src/transport.rs}` for telemetry changes.
- Preserve optional exporters, empty default features, provider flush/shutdown, and transport observation without exporter dependencies.
- Check service names, OTLP endpoint precedence, shared tonic middleware, and documented middleware limits.
- Queue metrics use bounded labels, never UUIDs, URLs, arbitrary source strings, or raw errors.
- Distinguish body bytes consumed from wire bytes, server receipt, and durable publication.
- Compare dependency pins, lockfiles, Gradle checksums, XcodeGen targets, and generated artifacts with affected source.
- Audit ignore rules for secrets, SDK paths, build outputs, caches, logs, and relative `.worktrees/` directories.
- Keep source, generated protobuf, lockfiles, and Gradle wrapper files visible to Git.
- Preserve persistent volumes, instance UUIDs, runtime keys, descriptor identity, and caller-owned teardown.

## Documentation Consistency

- Compare scope claims with current source and focused tests.
- Treat target boundaries as intended behavior until implementation evidence supports them.
- Preserve unrelated scaffolds; sibling repositories supply examples, not Screenroom runtime dependencies or policy authority.
- Keep transient validation results and approval records outside canonical prose.
