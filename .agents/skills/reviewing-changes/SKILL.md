---
name: reviewing-changes
description: Use for Screenroom change reviews to supply scaffold boundaries, platform risks, source consistency checks, validation commands, and evidence requirements to making-changes.
---

# Screenroom Review Inputs

This skill supplies repository-specific inputs to installed `making-changes`; it does not own the generic review workflow.

## Authority and Scope

- Read the [repository overview](../../../README.md), [documentation index](../../../docs/README.md), and [component outline](../../../docs/components.md).
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
| Observability | `crates/screenroom-observability/Cargo.toml` keeps telemetry and profiling dependencies optional, with no default features or runtime initialization. |
| Native clients | SwiftUI, Android Compose, and GTK4/libadwaita remain native shells, without core bridges, players, or adapter behavior. |
| Web SPA | `apps/web/vite.config.ts` sets the `index.html` fallback; `apps/web/src/routes/+layout.ts` disables SSR. |
| Static website | `website/vite.config.ts` has no SPA fallback; `website/src/routes/+layout.ts` enables prerender and disables client JavaScript. |
| Cloudflare Pages | Preserve `website/static/404.html` in `website/build/404.html` to prevent Pages' implicit SPA fallback. |
| PostgreSQL | `compose.yaml` binds localhost and uses local-only defaults; configuration validation does not authorize service startup. |
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
- Confirm that ignore rules retain source, generated protobuf output, lockfiles, and Gradle wrapper files.
- For new documentation, check whitespace directly because ordinary `git diff --check` excludes untracked files.
