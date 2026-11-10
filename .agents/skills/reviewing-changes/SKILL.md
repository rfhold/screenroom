---
name: reviewing-changes
description: Use for Screenroom change reviews to supply scaffold boundaries, platform risks, source consistency checks, validation commands, and evidence requirements to making-changes.
---

# Screenroom Review Inputs

This skill supplies repository-specific inputs to installed `making-changes`; it does not own the generic review workflow.

## Authority and Scope

- Read the [repository overview](../../../README.md), [documentation index](../../../docs/README.md), [component outline](../../../docs/components.md), and [backend guide](../../../docs/backend.md).
- Treat the component outline as stated scope, not an implementation claim.
- Preserve bare crates, empty contracts, and client shells unless the user explicitly requests implementation.
- Screenroom remains standalone; Waltr supplies pattern references, not runtime dependencies. Do not infer Moviee integration.
- Local review grants no commit, push, pull request, publication, deployment, or external mutation authority.
- Obtain explicit user approval for each such action. When requested, use conventional commits by scope without co-author trailers.

## Risk and Consistency Inputs

| Surface | Review inputs |
| --- | --- |
| Root Rust workspace | `Cargo.toml` selects `crates/*` and excludes `apps/linux`; preserve separate manifests and lockfiles. |
| Protobuf | Inspect `proto/`, `buf.yaml`, `buf.gen.yaml`, and `crates/screenroom-proto/src/generated/` together. Empty packages produce comments-only Rust output. |
| Observability | Dependencies remain feature-gated with no default features. Backend binaries enable shared telemetry setup and community `tonic-tracing-opentelemetry` middleware. Verify optional OTLP endpoints, service names, provider shutdown, and documented middleware limits. |
| Protobuf ownership | Catalog owns BrowseService, CatalogService, and SearchService; user owns UserService.GetCurrentUser; common owns SearchTypes/SearchMethods. Keep public contracts separate from private operator/internal contracts. Preserve the namespace break without preference aliases. Playback gateway stays empty. |
| Backend transport | Catalog and user expose persistent public reads through Envoy; operator/internal RPCs remain private. Playback gateway serves empty routes. No authentication verifier or live provider connection exists. |
| Client core and backend | Inspect public catalog/user/common contracts, `crates/screenroom-client-core/`, shared backend, catalog/user/worker, typed adapters, and fixtures together. Preserve native/WASM parity, immutable endpoint/user/timeout configuration, explicit scope, cancellation, bounded caches, and provenance. Public bindings exclude operator/internal RPCs and SQLx. |
| Local settings | Verify version-1 Foundation atomic JSON, Android validated SharedPreferences commit, Linux private atomic XDG JSON, and context-keyed web localStorage. Write failures must stay explicit; UUID selectors are not authentication. |
| Search | Require exact 1024-dimensional retrieval across all eligible source documents and single-snapshot coverage. Preserve source-document RRF before best-document canonical deduplication, explicit lexical degradation, query guards, and scope-bound pages. No ANN, lexical shortlist, reranker, or fake semantic success. |
| Ingest and artwork | Check durable leases/fences/checkpoints, atomic pages/follow-ups, immutable run policy, safe partial traversal, and complete-only removal reconciliation. Publish artwork only after verified required variants; preserve previous complete revisions, bounded streams, same-origin core fetches, and private object keys. |
| Native clients | SwiftUI, Android Compose, and GTK4/libadwaita remain native shells. Check core bindings, UI-thread isolation, request-generation guards, native focus, and network error states; no playback or real-provider integration. |
| Web SPA | `apps/web/vite.config.ts` sets the `index.html` fallback; `apps/web/src/routes/+layout.ts` disables SSR. |
| Static website | `website/vite.config.ts` has no SPA fallback; `website/src/routes/+layout.ts` enables prerender and disables client JavaScript. |
| Cloudflare Pages | Preserve `website/static/404.html` in `website/build/404.html` to prevent Pages' implicit SPA fallback. |
| PostgreSQL | Every database-backed service/init calls `db::connect_service` before listeners/work/readiness. Preserve advisory-lock startup migrations, short-lived DDL pools, restricted runtime pools, forward-only applied files, and runtime SQLx queries. Separate credentials do not isolate process privilege. |
| Shared local instance | `compose.yaml` publishes only loopback edge. Preserve volumes, Garage keys, instance UUID, create-only bootstrap state, descriptor identity, and caller-owned teardown. All E2E families attach to that instance; no standalone fixture API fallback. Configuration checks grant no lifecycle authority. |
| Dependencies | Compare manifests, lockfiles, package-manager pin, Gradle wrapper checksum, and XcodeGen targets against the affected source. |
| Documentation | Check scope claims and links against source; do not add inferred product features or one-off validation results. |

## Validation Inputs

Select checks for the affected scope. Prose-only changes require relevant link, factual, and whitespace checks, not every platform build.
Commands below start at the repository root unless a subsection specifies another directory.

### Rust, Protobuf, and Compose

```sh
cargo fmt --all -- --check
cargo test --workspace --all-features --locked
buf lint
buf generate
docker compose config --quiet
```

Rust requires an edition-2024-capable toolchain. Buf generation requires `protoc-gen-prost` and `protoc-gen-tonic` on `PATH`.
For protobuf changes, compare generated output with a pre-generation snapshot or the tracked baseline.
Run generation twice to check reproducibility. Confirm that neither run introduces unrelated output changes.
Compose requires Docker Compose; this check does not start containers.
Database integration tests require explicit disposable targets; never reuse the persistent development database for destructive setup.
Inspect each test's environment and ignore gate before execution. Live embedding requests require exact endpoint approval.
Do not reset volumes, reseed operator state, import providers, or rotate keys without separate explicit authority.

### Web SPA and Static Website

Use the Node engine range and pnpm pin from root `package.json`.
If the global pnpm launcher cannot bootstrap the pinned version, use these commands:

```sh
npx --yes pnpm@12.18.0 install --frozen-lockfile
npx --yes pnpm@12.18.0 check
npx --yes pnpm@12.18.0 build
```

The root scripts cover both pnpm packages. For isolated changes, use `--filter @screenroom/web` or `--filter @screenroom/website`.
After website builds, inspect `website/build/` for prerendered routes, the explicit 404, and absence of client JavaScript.

### Apple

From `apps/apple/`, use XcodeGen 2.44.1 or newer and Xcode with the relevant platform SDKs:

```sh
xcodegen generate
xcodebuild -project Screenroom.xcodeproj -scheme Screenroom-iOS -destination 'generic/platform=iOS Simulator' -derivedDataPath build/ios CODE_SIGNING_ALLOWED=NO build
xcodebuild -project Screenroom.xcodeproj -scheme Screenroom-tvOS -destination 'generic/platform=tvOS Simulator' -derivedDataPath build/tvos CODE_SIGNING_ALLOWED=NO build
xcodebuild -project Screenroom.xcodeproj -scheme Screenroom-macOS -destination 'generic/platform=macOS' -derivedDataPath build/macos CODE_SIGNING_ALLOWED=NO build
```

Review `project.yml` as source, not the generated Xcode project.
macOS GUI automation requires fresh approval and `SCREENROOM_MACOS_GUI_READY=1`; build authority alone does not grant desktop access.

### Android

From `apps/android/`, use a JDK compatible with the pinned Gradle/Android plugin and Android SDK platform 37.
Set `ANDROID_HOME` or an untracked `local.properties` SDK path.

```sh
./gradlew --no-daemon :mobile:assembleDebug :tv:assembleDebug :mobile:lintDebug :tv:lintDebug
```

### Linux

Use a Linux environment with pkg-config and GTK4/libadwaita development libraries plus their native dependencies.
The root Rust checks do not cover this workspace.

```sh
cargo fmt --manifest-path apps/linux/Cargo.toml --all -- --check
cargo check --manifest-path apps/linux/Cargo.toml --locked
```

## Evidence Expectations

- Report the affected files, commands, outcomes, and concrete blockers in the session, not canonical documentation.
- Distinguish passed, failed, and not-run checks. Treat unavailable platform checks as unverified.
- A successful scaffold build or zero-test run does not prove product behavior or test coverage.
- Before the first commit, inspect untracked files directly; a diff against `HEAD` cannot represent an unborn repository.
- Inspect staged and unstaged changes when a baseline exists.
- Audit ignore rules for secrets, local SDK paths, build output, caches, logs, and relative `.worktrees/` directories.
- Preserve prior dirty work and baseline evidence; `HEAD` alone does not identify delegate-owned changes.
- Check disk usage before large builds; preserve source, useful caches, and active diagnostics.
- Confirm that ignore rules retain source, generated protobuf output, lockfiles, and Gradle wrapper files.
- For new documentation, check whitespace directly because ordinary `git diff --check` excludes untracked files.
