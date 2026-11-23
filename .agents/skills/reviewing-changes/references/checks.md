# Screenroom Check Inputs

Commands start at the repository root unless a section names another directory.
Select checks for the affected surface; this reference does not prescribe a review sequence.
Inspect current scripts and manifests before execution.

## Local Static Checks

```sh
git diff --check
cargo fmt --all -- --check
python3 scripts/generate-client-models.py --check
node scripts/check-client-boundary.mjs
node --test scripts/test-client-boundary.mjs
buf lint
docker compose config --quiet
```

- Whitespace diffs omit untracked files; check their content directly.
- Check relative links and anchors in every changed instruction or Markdown file.
- Compose configuration checks do not start containers or grant lifecycle authority.
- Rust requires an edition-2024-capable toolchain.

## Rust and Database Checks

```sh
cargo test -p screenroom-queue --lib --locked
cargo test -p screenroom-catalog-adapter --lib --locked
cargo test -p screenroom-client-core --locked
cargo check -p screenroom-observability --no-default-features --locked
cargo test --workspace --all-features --locked
```

- Inspect test environment variables and ignore gates before database, provider, storage, or embedding tests.
- Database integration checks require explicit disposable targets, never the persistent development database.
- The workspace command is broad; do not use it as a substitute for target inspection.
- Live provider and embedding requests require explicit endpoint authority.
- Builds and tests can consume substantial disk space; preserve useful caches and active diagnostics.

### Attached Acceptance

Use the [canonical attached-test procedures](../../../../docs/testing.md#attached-rust-acceptance) for exact commands and prerequisites.
Inspect `crates/screenroom-backend-tests/tests/roundtrip.rs` and `crates/screenroom-client-core/tests/persisted_backend.rs` before selection.
Backend empty-installation and indexed-source cases have mutually exclusive prerequisites; select exactly one case with `--ignored --exact NAME`.
The shared-core populated case requires real indexed Jellyfin content and published fetchable artwork; no artwork skip exists.
Both test targets require explicit endpoint/user selectors; backend roundtrips additionally require Python 3, grpcurl, and Buf.
Attached reads need approval for the exact caller-owned installation; disposable database authority does not cover them.
Local assertion tests do not prove attached, browser, live TLS, or container credential-volume acceptance.

## Generation and Bindings

```sh
buf generate
python3 scripts/test-patch-swift-cancellation.py
python3 scripts/test-build-client-core.py
node --test scripts/test-client-cache.mjs scripts/test-web-core-cache.mjs
```

- Buf needs `protoc-gen-prost` and `protoc-gen-tonic` on `PATH`.
- `buf generate` rewrites tracked generated Rust; inspect dirty output before invocation.
- Compare generation with the baseline; a second run must produce no additional changes.
- Inspect `scripts/build-client-core.sh` for artifact modes, SDK requirements, and generated output boundaries.
- UniFFI uses native library metadata; WASM requires the CLI version from `Cargo.lock`.
- Apple and Android preparation can build core artifacts; do not hand-edit those outputs.
- Cancellation checks complement, rather than replace, native buffer and future lifecycle smoke checks.

## Web and Website

```sh
pnpm --filter @screenroom/web check
pnpm --filter @screenroom/web build
pnpm --filter @screenroom/website check
pnpm --filter @screenroom/website build
(cd apps/web && pnpm exec playwright test --config e2e/maintenance.config.ts)
```

- Use the Node range and pnpm pin from root `package.json`.
- Web precheck/prebuild scripts prepare WASM artifacts; they are not prose-only checks.
- Verify static website routes, explicit output 404, and absence of client JavaScript after its build.
- E2E runners can attach to services and operate browsers or devices; inspect their targets before authorization.
- Package scripts do not authorize provider imports, service startup, credential rotation, or persistent-state resets.

## Apple

From `apps/apple/`, with XcodeGen and the appropriate Xcode SDKs:

```sh
xcodegen generate
xcodebuild -project Screenroom.xcodeproj -scheme Screenroom-iOS -destination 'generic/platform=iOS Simulator' -derivedDataPath build/ios CODE_SIGNING_ALLOWED=NO build
xcodebuild -project Screenroom.xcodeproj -scheme Screenroom-tvOS -destination 'generic/platform=tvOS Simulator' -derivedDataPath build/tvos CODE_SIGNING_ALLOWED=NO build
xcodebuild -project Screenroom.xcodeproj -scheme Screenroom-macOS -destination 'generic/platform=macOS' -derivedDataPath build/macos CODE_SIGNING_ALLOWED=NO build
```

`project.yml` owns target configuration. XcodeGen's pre-generation hook prepares core artifacts.
macOS GUI automation needs fresh explicit approval and `SCREENROOM_MACOS_GUI_READY=1`.

## Android

From `apps/android/`, with the pinned Gradle-compatible JDK and Android SDK:

```sh
./gradlew --no-daemon :mobile:assembleDebug :tv:assembleDebug :mobile:lintDebug :tv:lintDebug
```

Inspect Gradle SDK pins and `apps/android/scripts/build-core.sh` for JNI generation prerequisites.
Keep SDK paths in the environment or untracked `local.properties`.
Build authority does not grant device instrumentation authority.

## Linux

Use Linux with GTK4/libadwaita development libraries and their native dependencies:

```sh
cargo fmt --manifest-path apps/linux/Cargo.toml --all -- --check
cargo check --manifest-path apps/linux/Cargo.toml --locked
```

The root workspace excludes Linux. Compositor E2E checks require their own environment and explicit target scope.
