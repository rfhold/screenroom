# Testing

This guide separates isolated contract tests from real-source and platform acceptance.
A test command does not grant provider, database, Docker lifecycle, credential, SDK/device, or GUI authority.
The [backend guide](backend.md) owns lifecycle operations; the [Jellyfin guide](jellyfin.md) owns managed setup and credentials.

## Test layers

| Layer | Purpose | Boundary |
| --- | --- | --- |
| Rust unit tests | DTO validation, queue decisions, provider parsing, private-file access, and cancellation. | Synthetic records and isolated filesystem state. |
| Local HTTP fault tests | Authenticated probes, malformed/partial responses, status classification, DNS/redirect guards, timeouts, and body limits. | Loopback test servers, not product provider adapters. |
| Disposable PostgreSQL tests | Transactions, leases/fences, retries, redrive, publication, budgets, and managed connection grants. | Explicit disposable target; never the persistent development database. |
| Runtime/orchestration tests | Command construction, descriptor identity, ownership, process cleanup, and compose contracts. | Stub executables and temporary state, not Docker activation. |
| Client-boundary checks | Forbidden local catalog/artwork and direct provider-flow patterns. | Static production-source checks, not proof of all network traffic. |
| Real-source acceptance | Managed discovery/save, provider identity, queue execution, catalog reads, and private artwork. | Exact approved Jellyfin and installation targets. |
| Platform integration | Bindings, packaging, focus/navigation, real-source assertions, cancellation, and truthful errors. | Required platform SDK, simulator/device, or browser. |

Isolated HTTP, filesystem, and PostgreSQL fault support remains legitimate test infrastructure.
It must not become a runtime mock adapter, bundled catalog, fixture server, or fallback product data source.
Tests must not substitute fixture titles for source-data assertions.
Historical website captures remain illustrations; they prove neither current provider integration nor playback.

## Local checks

From the repository root, these commands select source-defined checks:

```sh
cargo fmt --all -- --check
cargo test --locked -p screenroom-catalog-adapter --lib
just backend-test
node --test scripts/test-e2e-orchestration.mjs
pnpm check:client-boundary
pnpm test:client-boundary
```

Rust tests create build outputs and can use isolated local HTTP/filesystem state.
`just backend-test` runs stubbed shell checks, lifecycle tests, and offline packaging checks.
It uses Bash, `jq`, Python 3, and PyYAML `6.0.3`.
The lifecycle and orchestration tests check construction and isolation; they do not establish a live Compose deployment.
The [client-boundary guard](../scripts/check-client-boundary.mjs) checks production source/resources across all four client families.
Its [tests](../scripts/test-client-boundary.mjs) verify detection rules; test support and the separate website remain outside the production scan.
Static checks do not inspect packaged artifacts or prove complete runtime data flow.

Broader Rust checks use `cargo test --workspace --all-features --locked` after test-target and resource review.
Database integration tests require explicit disposable PostgreSQL configuration and their source-defined ignore gates.
Some database setup resets schemas. Never substitute the persistent development database.
Live embedding tests require separate approval for the exact endpoint, model, and credential boundary.
Ordinary test success does not establish live provider or exporter-to-collector behavior.

For telemetry tests, use a local or intercepted collector. The Web `telemetry.spec.ts` fixture checks that Faro emits the span carried on the real WASM Home request. The observability crate checks the backend's `rpc.server.call.duration` histogram against final trailers. Keep Compose export off for isolated tests with `SCREENROOM_TELEMETRY_DISABLED=1`.

## Playback checks

The [playback contract](playback.md) owns product behavior; these checks use isolated support rather than a product provider or catalog.
From the repository root:

```sh
cargo test --locked -p screenroom-catalog-adapter --lib
cargo test --locked -p screenroom-catalog-adapter --test playback_protocol
cargo test --locked -p screenroom-backend --lib
cargo test --locked -p screenroom-user --lib
cargo test --locked -p screenroom-playback-gateway
cargo test --locked -p screenroom-client-core
buf lint
python3 scripts/generate-client-models.py --check
python3 scripts/test-generate-client-models.py
just backend-test
pnpm --filter @screenroom/web test:playback
node --test apps/web/tests/watched.test.mjs
```

The [adapter suite](../crates/screenroom-catalog-adapter/src/jellyfin/playback/tests.rs) covers device authorization, protocol faults, resource guards, and cleanup.
The [gateway suite](../crates/screenroom-playback-gateway/src/tests.rs) covers isolated control/media boundaries and shutdown task ownership.
The [core suite](../crates/screenroom-client-core/tests/playback.rs) covers public transport, deadlines, cancellation, validation, and profile-generation fences.
Default Rust commands leave ignored database and attached/live cases unrun.
Do not add an unfiltered `--ignored` selector.

`test:playback` runs [controller regressions](../apps/web/tests/playback.test.mjs) and [mounted browser cases](../apps/web/tests/playback-browser.test.mjs).
Its pretest hook invokes [`prepare-core.mjs`](../apps/web/scripts/prepare-core.mjs), which checks or rebuilds the owned WASM artifact.
Prerequisites include workspace dependencies, the repository-pinned pnpm, the WASM toolchain, Playwright Chromium, and `ffmpeg` with `libx264` and AAC.
The browser suite generates small owned MP4/HLS clips and serves isolated HTTP/gRPC-Web responses on distinct loopback SPA/API origins.
It exercises actual media decode and anonymous WebVTT through real WASM exports.
Its HTTP double does not establish positive gateway-plus-Jellyfin acceptance.
The watched suite runs separately; `test:playback` does not select it.

`node --test apps/web/tests/native-navigation.test.mjs` starts an owned isolated Vite server and exercises the actual SvelteKit router with a client fixture.
It covers native deep routes, legacy canonicalization, invalid scope, search-page restoration, player Back/Forward/reload without automatic Create, and disabled Maintenance.
The fixture does not prove media decode; the separate mounted playback suite owns that check.

Wire/model changes also require Buf generation, model drift checks, WASM preparation, and native binding checks.
Use the [core build modes](client-api.md#build-the-core) for `bindings` and `wasm`.
Generated Swift/Kotlin output alone does not prove native SDK builds or application behavior.
Web type/build checks can regenerate WASM; keep generation ownership explicit.
Safari native HLS requires separate browser acceptance; Chromium MSE success does not establish Safari support.

### Player presentation checks

The [shared presentation contract](playback.md#shared-player-presentation) defines expected behavior, not a cross-platform acceptance result.
Source review and isolated presentation tests cover layout structure, visibility state, menu dismissal, and timer teardown.
They do not prove actual pointer, touch, keyboard, remote, subtitle, or native-window behavior.
Use the [Web playback checks](#playback-checks) and [native platform procedures](#native-playback-checks) for reproducible entry points.

With separate platform/runtime approval, check:

- Stable video bounds through preparation, errors, menus, controls, and fullscreen transitions.
- Contain sizing and usable controls across narrow, desktop, and TV viewports.
- Accessible icon labels, target sizes, selected states, and visible keyboard/remote focus.
- Three-second idle fade only during active playback; pause, errors, menus, scrubbing, and control focus retain overlays.
- Reveal gestures without accidental playback actions, invisible focus targets, or hidden overlays that intercept clicks.
- Click/touch/focus menu access, bounded track lists, subtitles Off, and menu dismissal before exit.
- Readable subtitles with visible overlays.
- Stable media surface and session/report sequence through presentation-only transitions.

Native GUI/device, decoder, scene, fullscreen, and real-provider acceptance remain separate evidence boundaries.
Presentation checks grant no backend lifecycle, provider-write, or GUI automation authority.

### Native playback checks

Platform guides own the commands and prerequisites:

- [Apple native playback](../apps/apple/README.md#native-playback) documents isolated lifecycle tests without a backend or GUI.
- [Android build](../apps/android/README.md#build) documents unit tests, mobile/TV APK builds, lint, and instrumentation compilation.
- [Linux native run](../apps/linux/README.md#native-run) documents native checks and the separate headless library target.

The [Apple tests](../apps/apple/Tests/NativePlaybackTests.swift) exercise controller lifecycle and exact mutation retries.
Apple source routes tvOS Play/Pause through report-aware controls and stops on both inactive and background scenes.
Physical remote input, actual scene transitions, and fullscreen continuity require separate platform acceptance.
Apple catalog-state checks exercise bounded snapshots and stale-read fences; isolated success does not prove runtime scroll or focus restoration.
The [Android tests](../apps/android/ui/src/test/kotlin/org/screenroom/ui/NativePlaybackTest.kt) exercise player ownership, reports, retries, and cleanup.
Android Stop checks distinguish local termination from provider confirmation, exact-request recovery, and recovery expiry.
Fullscreen and Back require separate device checks for presentation continuity without new sessions or altered event sequences.
The [Linux headless tests](../apps/linux/src/playback_state.rs) exercise offsets, pre-origin gateway seeks, exact logical targets, final-position freeze, and report order.
These isolated checks do not establish real-provider media decode or platform GUI behavior.
APK/instrumentation compilation does not execute device tests.
Apple SDK builds do not execute simulator or device playback.

Linux native checks require GTK4, libadwaita, Pango, GdkPixbuf, GStreamer development libraries, and runtime plugins.
Required media components include `playbin`, `gtk4paintablesink`, HTTP, MP4 demux, H.264/AAC decoders, and applicable HLS/subtitle plugins.
The [Linux playback guide](../apps/linux/README.md#native-playback) defines capability detection and runtime error behavior.
The [Linux prerequisite guide](../apps/linux/README.md#native-run) names the Debian packages for these factories.
The [compositor Dockerfile](../apps/linux/tests/e2e/Dockerfile) installs GTK4 sink, libav, bad, X/text-overlay, and inspection-tool packages.
It also copies the [plugin check](../apps/linux/scripts/check-playback-plugins.sh) into the image as `screenroom-check-playback-plugins`.
The script checks factory availability without a playback pipeline or registry-cache updates.
Factory discovery does not prove decode, subtitle synchronization, or audio/video output.
Dockerfile source alone does not establish a container build, installed factory availability, native linkage, or runtime acceptance.
`DOCS_RS=1` bypasses native dependency probes; it provides no genuine native build, linkage, plugin, or runtime acceptance.
After a bypass check, use a fresh Cargo target directory for native checks with `DOCS_RS` unset.
Headless `--no-default-features --lib` checks exclude the GTK/GStreamer application.
Root workspace checks also exclude the independent Linux workspace.

Native acceptance requires separate approval for actual devices or GUI sessions and provider-write effects.
Exercise platform exit/background hooks, focus, fullscreen transitions, subtitle synchronization, audio choices, and actual decoder failures separately.
Also verify explicit Play after restoration, fixed final positions, capability privacy, and uncertainty without duplicate personal reports.
The [native lifecycle contract](playback.md#native-stop-and-restoration) records source differences, not cross-platform runtime signoff.

### Playback database boundary

The [repository regressions](../crates/screenroom-backend/tests/playback.rs) and [gateway database cases](../crates/screenroom-playback-gateway/src/tests/database.rs) require `SCREENROOM_PLAYBACK_TEST_DATABASE_URL`.
These ignored cases require a separately authorized, disposable PostgreSQL target and can reset database schemas.
Never guess a URL or substitute the persistent installation.
Compile-only checks require no database connection:

```sh
cargo test --locked -p screenroom-backend --test playback --no-run
cargo test --locked -p screenroom-playback-gateway --no-run
```

Database test execution remains separate from compilation and source review.
Only executed disposable-target checks establish migration, grant, transaction, and race behavior.

### Real playback acceptance

Obtain separate approval for the exact installation, Jellyfin server, personal profile, actual titles, lifecycle changes, and provider-write effects.
Confirm the [runtime prerequisites](backend.md#playback-gateway-runtime) without a private-volume reset or implicit Quick Connect enablement.
Discover approved real Movie/Episode IDs and source/library scope; do not substitute generated clips or fixture catalog entries.
The acceptance cases include:

- Actual direct MP4 and HLS decode through the public gateway.
- Pause, seek, live personal resume, audio selection, WebVTT, and supported burn-in.
- Correct personal-user attribution for start, progress, and stop; no indexing-profile fallback.
- Exact encoding/device cleanup after stop, replacement, expiry, and restart.
- Profile/source revocation, media ranges, anonymous cross-origin requests, and capability privacy.
- Safe ambiguous outcomes with no duplicate start report or stale watched publication.
- Actual image/configuration load, private mount ownership, restricted grants, and startup readiness.

Keep native engine acceptance separate from shared-binding support and isolated controller tests.
Keep Safari, provider codecs/subtitles, TLS, actual runtime configuration, and database acceptance as distinct evidence boundaries.
No scripted procedure here enables provider features or resets private state.

## Real-source acceptance

Before provider acceptance, identify the exact approved installation, Jellyfin origin, administrator key channel, and selected local user.
Record suitable supported libraries and an enabled Jellyfin profile outside public artifacts.
For content flows, provide actual movies or series with episodes, useful searchable text, and provider artwork where available.
For watched tests, provide explicit played/unplayed observations and a current personal profile link.
Tests must discover actual public item/library IDs and source content rather than assume canned titles or fixed library identifiers.

The acceptance boundary includes:

- A healthy zero-source catalog before connection setup.
- Authenticated discovery without persistence or queue work.
- Fresh save-time server/profile/library checks with the exact key.
- Save alone without jobs; explicit Save and Refresh with atomic initial run admission.
- Dynamic user/worker access without manual provider JSON or restart.
- Complete metadata/artwork outcomes through the shared queue, including safe partial failures and terminal redrive.
- Ordinary client reads and private artwork transport without key/locator disclosure.
- Current watched state and personal links, separate from catalog delta checkpoints.
- Actual credential-volume ownership, modes, read-only mounts, and restricted database grants.

Web setup uses shared client gRPC-Web. Isolated browser tests exercise that path with controlled local responses.
Those tests do not establish attached real-source acceptance or a live provider deployment.
No source-only check establishes Docker startup/restart, actual volume ownership, SDK/device behavior, or provider E2E acceptance.
If a required dependency or data case is absent, report the limitation rather than fabricate success.
An empty catalog can pass readiness but cannot prove populated browse, artwork, hierarchy, or search flows.

## Attached Rust acceptance

These ignored tests attach to a caller-owned installation; they do not provision a backend or import provider content.
Obtain approval for the exact installation and read effects before execution.
Use `SCREENROOM_USER_ID` for an actual enabled local user, not a demo UUID or authentication token.
Set `SCREENROOM_API_ENDPOINT` explicitly to `http://127.0.0.1:<port>`, with a port from 1024 through 65535.
If `SCREENROOM_E2E_API_ENDPOINT` exists, keep it identical to that endpoint.
The [backend roundtrip tests](../crates/screenroom-backend-tests/tests/roundtrip.rs) also require Python 3, `grpcurl`, and Buf on `PATH`.
Their subprocess invokes the [backend contract helper](../scripts/backend-contract.sh) for native/gRPC-Web and private-route checks.
It uses the supplied user selector, not a substitute installation identity.

### Empty installation

Select this case only when the installation has zero managed connections, libraries, indexed items, and artwork.
The case checks truthful empty reads, absent branding, readable Home rows, cancellation, invalidation, and the transport contract.
From the repository root, with the endpoint and user variables set:

```sh
cargo test --locked -p screenroom-backend-tests --test roundtrip -- --ignored --exact empty_installation_native_core_roundtrip
```

### Indexed source

Select this case only after a managed real Jellyfin source supplies indexed content and published, fetchable artwork.
The case discovers actual IDs and requires a library item with artwork; absent artwork fails rather than skips.
It checks scoped item/detail/artwork agreement, nonempty artwork bytes, cache invalidation, cancellation, readable Home rows, and the transport contract.
Each paginated traversal requires completion within 512 pages and 10,000 entries.
The whole case has a 120-second deadline, including its bounded contract subprocess.

```sh
cargo test --locked -p screenroom-backend-tests --test roundtrip -- --ignored --exact indexed_source_native_core_roundtrip
```

The empty and indexed prerequisites are mutually exclusive.
Select exactly one named case for an installation state; never run both through an unfiltered `--ignored` invocation.
Neither case authorizes setup, imports, schema resets, or service lifecycle changes.

### Shared-core populated reads

The [persisted-backend case](../crates/screenroom-client-core/tests/persisted_backend.rs) also requires explicit endpoint/user variables and a real indexed Jellyfin source.
It requires a library, an item, and published fetchable item artwork; no artwork skip or synthetic fallback exists.
Branding remains optional, but every returned branding record requires a valid identity and fetchable nonempty bytes.
Search requests follow advertised capabilities; semantic fallback requires a documented degradation reason only when the request includes semantic search.
Home row read errors fail the case. The whole case has a 120-second deadline.

```sh
cargo test --locked -p screenroom-client-core --test persisted_backend -- --ignored --exact persisted_backend_native_roundtrip
```

Local assertion and pagination tests do not establish attached acceptance.
These attached reads do not prove Web onboarding, live TLS validation, or actual container credential-volume ownership.

## Client E2E commands

After approval for the exact runtime and platform effects, select families from the repository root:

```sh
bash scripts/e2e.sh
bash scripts/e2e.sh apple web
bash scripts/e2e.sh --category retry apple web
bash scripts/e2e.sh --category happy-path
```

Categories are `happy-path`, `retry`, `regression`, and `all`.
CLI category overrides `E2E_CATEGORY`; the default is `all`, except showcase updates default to `happy-path`.
Unknown or empty categories fail before preparation.
These commands invoke platform tests; their existence does not prove that every suite supports every real-source scenario.
The [runner](../scripts/e2e.sh) prepares fresh Apple, Android, and WASM products sequentially before family suites.
Content fingerprints allow cache reuse; suite checks reject stale products instead of concurrent regeneration.
`SCREENROOM_CORE_PREPARED=1` selects internal verification, not a build bypass.
Apple simulator, Android, Linux, and Web suites then run concurrently.
Apple and Android profiles stay sequential within each family; Web uses one Playwright worker.
Absent tools fail rather than silently skip a family.

The root runner owns one named persistent Compose instance unless `SCREENROOM_BACKEND_ATTACH=1` selects caller-owned attachment.
An active instance requires explicit attach mode; standalone family runners never create substitute APIs.
With separate approval, attach without teardown:

```sh
SCREENROOM_BACKEND_ATTACH=1 bash scripts/e2e.sh
```

Both `SCREENROOM_API_ENDPOINT` and `SCREENROOM_E2E_API_ENDPOINT` must match the descriptor's public origin.
The default is `http://127.0.0.1:50051`; `SCREENROOM_BACKEND_PORT` selects an explicit alternate port.
Every family uses the same secret-free descriptor and instance UUID.
The runner obtains `SCREENROOM_USER_ID` from the installation descriptor, not a fixed demo UUID.
That UUID and unique local-settings namespaces provide selection/isolation, not authentication.
Android uses same-port ADB reverse. Linux joins the Compose client network through a byte-only loopback forwarder.
The owner lock rejects concurrent root runs. Cleanup stops only invocation-owned process trees and runtime resources.
Ordinary backend teardown preserves volumes, private keys, users, and installation identity.

### Platform prerequisites and limits

Install platform prerequisites first; the runner installs no SDKs, packages, or browsers.
Root orchestration requires Node.js, including native-only selections; root `package.json` owns its version range.
Web requires pnpm `12.18.0`, workspace dependencies, and Playwright Chromium.
If the global launcher selects another pnpm version, use `npx --yes pnpm@12.18.0` for package commands.
Native prerequisites include the selected platform SDKs; Android uses JDK 17, SDK 37, and an AVD.
Apple requires Xcode/XcodeGen; Linux headless integration requires Docker. Orchestration requires GNU timeout.

Apple defaults to iOS, iPadOS, and tvOS simulators.
macOS XCUITest requires fresh desktop approval and `SCREENROOM_MACOS_GUI_READY=1`.
After that approval, `E2E_MACOS=1 bash scripts/e2e.sh apple` includes macOS before concurrent families.
The standalone macOS selector is `bash apps/apple/scripts/e2e.sh macos`, with the same readiness variable.
`E2E_MACOS` accepts only `0` or `1`; unattended batches leave it disabled.
Toolchain builds and binding generation do not establish simulator, device, or provider integration acceptance.

| Guide | Platform-specific contract |
| --- | --- |
| [Apple](../apps/apple/README.md) | Packaging, simulator profiles, GUI approval, and captures. |
| [Android](../apps/android/README.md) | Mobile/TV builds, emulator routes, and instrumentation. |
| [Linux](../apps/linux/README.md) | Independent GTK workspace and headless Wayland integration. |
| [Web](../apps/web/README.md) | WASM preparation, browser suites, and public-client setup. |

## Artifacts and showcase exports

The runner retains per-family logs, exit status, category, and measured durations under `build/e2e/runs/`.
Each `.summary` records those fields; GNU timeout bounds family suites to 40 minutes with a 20-second kill grace.
Preserve useful failure logs and captures; avoid credentials, private URLs, key files, and raw upstream responses in shared artifacts.
The [backend contract helper](../scripts/backend-contract.sh) compares native/gRPC-Web reads and public/private route separation on an approved live instance.
It accepts an empty catalog and does not establish populated provider acceptance or semantic/artwork completeness.

With separate authority for website file changes, the default browse showcase export uses:

```sh
bash scripts/e2e.sh --update-showcase
bash scripts/e2e.sh --update-showcase apple web
```

Browse exports require `happy-path` or `all`; `retry` and `regression` fail before tests.
Every selected suite and PNG validation must succeed before website files change; unselected families retain their images.
Exports require `file`; Apple attachment selection also requires `jq`.
Cancellation or export failure restores selected prior images. A failed restoration preserves backups and reports their path.
Showcase export expects Home, sidebar, library, search, and detail captures; a happy-path tag alone does not produce them.
Missing current captures fail export; static screenshots establish no provider or playback capability by themselves.
Native paths use `{platform}/{home,sidebar,library,search,detail}.png` under `website/static/showcase/`.
Web Home uses `web/desktop.png` and `web/mobile.png`; other Web flows use `web/{desktop,mobile}-{sidebar,library,search,detail}.png`.
Apple export requires exactly one current-run named `{flow}-success` attachment per flow/profile.
Default Apple exports preserve macOS images; explicit approved `E2E_MACOS=1` includes that profile's captures.
The other suites clear prior success captures before execution.
Showcase export remains a separate local file mutation, not external publication authority.

For real Jellyfin playback evidence, use the [playback showcase procedure](../website/README.md#playback-journey).
That journey requires explicit backend attachment and title selectors, accepts only `happy-path`, and preserves historical browse images.
