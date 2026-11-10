# Apple catalog shell

The iOS, iPadOS, macOS, and tvOS apps share native SwiftUI views and an isolated dark theme.
`Sources/Home/Catalog.swift` adapts Rust UniFFI domain records for presentation.
The Rust client core owns Browse, Catalog, Search, User, and artwork requests to the
[persistent local backend](../../docs/client-api.md). Home rows, library membership, hierarchy, credits,
rights, and search results come from that API; all paginated reads follow continuation tokens.
Preferences use version-1 Foundation atomic JSON in Application Support, not a server preference service.
Legacy UserDefaults migration runs only when no settings file exists; persistence errors retain choices and expose a save-retry action.
The Data & artwork credits disclosure reads bounded `CatalogInfo` notices and branding through the core.
It enumerates complete paginated `ListArtwork` credits on demand, independently of visited Home rows, without eager global image downloads.
Server notices retain their order, sections, text, and links. Server provenance notes appear without title-specific branches.
All artwork bytes, including provider logos, load through the core with explicit loading and failure states.
The app contains no bundled catalog JSON, media, portraits, or provider branding. Native app icons and navigation glyphs remain local.
There is no playback,
authentication, connected media provider, or synchronization.

## Core and development endpoint

The [wire contract](../../docs/client-api.md) places catalog RPCs in `screenroom.catalog.v1` and UserService in `screenroom.user.v1`.
Immutable UniFFI `ClientConfig` contains endpoint, user UUID selector, and timeout. Source/library scope accompanies each scoped read.

Run `xcodegen generate` from `apps/apple/`. Its preparation hook builds missing or stale
bindings and the eight-architecture XCFramework with `scripts/build-client-core.sh apple`
at the repository root. This requires the Rust/Apple toolchains documented in the API guide.
Generated Swift and the XCFramework are referenced directly under ignored `target/client-core/`;
do not copy or commit them into the app. The preparation cache hashes source and dependency inputs,
profile and target options, resolved active/nightly toolchains, Xcode/compiler/SDK identities,
and generated products. Changed inputs or replaced products trigger preparation again.

For manual runs, use an approved persistent instance from the [backend guide](../../docs/backend.md#persistent-local-instance).
The app defaults to `http://127.0.0.1:50051`. Override it with `SCREENROOM_API_ENDPOINT`
or the launch arguments `--SCREENROOM_API_ENDPOINT http://127.0.0.1:<port>`.
`SCREENROOM_USER_ID` (or the matching launch argument) selects an enabled local UUID; it is not authentication.
`SCREENROOM_E2E_SETTINGS_NAMESPACE` isolates local settings files, not backend users.
The core uses a five-second request timeout. Before core construction, Swift validates the endpoint:
Debug permits HTTP only for localhost, 127.0.0.1, and ::1; Release requires HTTPS.
Credentials, query strings, fragments, and non-root paths are rejected. Invalid configuration
shows an error instead of constructing a network client. This check applies to Rust sockets,
independently of ATS. macOS also enables sandbox client networking.
Run `bash apps/apple/scripts/test-endpoint-policy.sh` from the repository root to test both policies
without launching the Mac GUI.

## Interface

Home presents server-defined horizontal poster rows. The persistent sidebar expands in the layout;
on narrow windows and phones it becomes an inline top rail and expandable panel.
Libraries contain movies and series; series open seasons, then episodes.
Detail pages show server artwork, cast/crew roles and portraits, title rights, provider references, and rating snapshots.
Provider identity, status, links, rating scale, vote counts, palette, and metadata dates come from server records.
Missing vote counts remain distinct from zero votes. Missing colors use a neutral placeholder palette.
The detail header uses two columns at 640 logical points of available content width.
Narrow layouts stack a 160×240 poster above the summary; tablet and desktop posters use 200×300, and TV posters use 240×360.
The provider row precedes the synopsis. Cast, Crew, and Title rights span the content width below the header.
Adaptive people grids preserve full names, roles, initials, and character-still labels.
Global Data & artwork credits
include the exact TMDB notice and approved logo, every artwork author/license/source,
crop information, and edition/jurisdiction-specific film rights. Sprite Fright keeps its official
2021 year separately from TMDB's 2024 date. Back and breadcrumb buttons
return to the originating library, home, or search context.

Search uses stable server library IDs and titles and waits 350 ms after query edits. Local settings supply selected types.
Search capabilities supply available sources, types, methods, query length, and the semantic description.
Protocol label mappings only format those capabilities; they do not add choices.
All catalog, item-type, Text, Fuzzy, Semantic, and Title only
controls live inside initially closed Settings. Query, Clear, and Settings remain outside.
Provider choices intersect the installation source, library, types, and query; they do not establish a live provider connection.
Semantic uses exact backend vector search. The interface displays explicit lexical degradation when semantic coverage or availability fails.
At least one type and method stays selected.
Title only disables Semantic; turning it off does not select Semantic again.
The sidebar's Search titles action starts a fresh search, even from an existing search:
it clears the query, restores All catalogs and all methods, turns Title only off, and closes Settings.
Selected item types persist in local settings. Detail breadcrumbs and Back preserve the active
query, catalog filter, item types, methods, title-only state, and library scope instead.
Home uses All libraries, so search from Home covers every library. Search from a library retains
that library's scope, and its Back button restores the prior hierarchy. Other search state remains in memory.
Subtitles save Off, Library tracks, or Transcription locally; they do not enable playback.
Refresh invalidates core reads and reloads active catalog, search, provenance, and artwork tasks.
The status reports success or failure only after the active catalog/search request completes.
The app has no Log out, Welcome, or Continue authentication flow.
On tvOS, sidebar destinations use uniform 56-point rows with separate selected and focused
appearances. Subtitles starts as a closed disclosure with its current value.
A choice of Off, Library tracks, or Transcription closes the disclosure. Subtitles and Refresh
remain in a footer while library navigation can scroll. Expansion pushes the content.

## E2E tests

Use Xcode with the required simulator runtimes and XcodeGen 2.44.1 or newer.
Install GNU coreutils and jq with `brew install coreutils jq`.
The runner requires Node.js, Bash 3.2 or newer, and GNU `timeout` or `gtimeout` on `PATH`.
The runner requires the shared descriptor-backed backend and verifies real readiness and instance identity.
It never starts or stops an API server. `SCREENROOM_API_ENDPOINT` must match the descriptor origin and endpoint alias.
Each test injects that endpoint, the explicit local `SCREENROOM_USER_ID`, and an isolated local settings namespace.
Regression tests request one-item pages
with `SCREENROOM_API_PAGE_SIZE=1` to exercise continuation handling. Startup, server output,
readiness, and cleanup logs live in the run directory.
The regression and all categories also execute headless catalog-state and endpoint-policy regressions before platform tests. They cover delayed/failed
detail transitions, nullable vote counts, variant provider/scale/palette/capability records, and Library/Search-first provenance with one-item API pages.
The headless tests also retrieve the server logo through the actual core and verify PNG bytes with native image decode.
The headless endpoint-policy check requires no server or GUI.
Run these commands from the repository root:

```sh
bash apps/apple/scripts/e2e.sh       # iOS, iPadOS, and tvOS
E2E_MACOS=1 bash apps/apple/scripts/e2e.sh  # All four Apple profiles
bash apps/apple/scripts/e2e.sh ios
bash apps/apple/scripts/e2e.sh ipad
bash apps/apple/scripts/e2e.sh tvos
bash apps/apple/scripts/e2e.sh macos
```

macOS XCUITest requires fresh user approval and `SCREENROOM_MACOS_GUI_READY=1` because it uses the desktop.
After approval, prefix a macOS command with that readiness variable. The explicit `macos` profile runs only macOS,
even when `E2E_MACOS` is unset or `0`. For default/all runs, `E2E_MACOS=1` adds macOS;
unset or `0` leaves it disabled. Other values fail before generation, builds, or tests.
The root runner uses the same setting: `E2E_MACOS=1 bash scripts/e2e.sh apple` runs all four profiles.

### Categories

`E2E_CATEGORY` selects exact Xcode test methods. Standalone runs default to `all`.
Unknown or empty categories fail before server startup, preparation, or builds.

| Category | Tests | Successful screenshots |
| --- | --- | --- |
| `happy-path` | `testHappyPathShowcase`: one app launch, five canonical flows, default preferences and 100-item pages. | Five per profile. |
| `retry` | `testCoreUnavailableAndRetry`: unavailable endpoint, failed Retry without fallback, then healthy-endpoint app relaunch and core artwork recovery. | None. |
| `regression` | `testHomeCatalogAndNavigation`, `testFreshSearchAndHomeScope`, `testSearchPaginationAndPreferencesRoundTrip`; tvOS also runs `testSidebarRemoteControls`. Includes headless catalog-state and endpoint-policy checks. | None. |
| `all` | Every category, without duplicate success attachments. | Five per profile. |

For screenshot-only checks, use `E2E_CATEGORY=happy-path bash apps/apple/scripts/e2e.sh all`.
Use `E2E_CATEGORY=retry` or `E2E_CATEGORY=regression` with the same command for separate checks.
The root showcase exporter defaults to `happy-path`; ordinary root checks default to `all`.
`scripts/test-category.sh` owns the method mapping. Add future categories there and document their coverage here.
Every category retains core-product fingerprint verification before Xcode generation.
`bash apps/apple/scripts/test-category-contract.sh` checks category defaults, exact mappings, early rejection,
and capture preservation with poisoned build, device, and network commands.

The runner selects an available simulator for each device family and prefers a booted simulator.
It never selects a physical device.
To select an exact simulator, use these optional environment variables:

```sh
E2E_IOS_DESTINATION='platform=iOS Simulator,id=<IPHONE_UUID>' bash apps/apple/scripts/e2e.sh ios
E2E_IPAD_DESTINATION='platform=iOS Simulator,id=<IPAD_UUID>' bash apps/apple/scripts/e2e.sh ipad
E2E_TVOS_DESTINATION='platform=tvOS Simulator,id=<APPLE_TV_UUID>' bash apps/apple/scripts/e2e.sh tvos
```

Use `xcrun simctl list devices available` to find simulator UUIDs.
Overrides require the exact destination format above and an available simulator from the correct family.
The macOS destination is `platform=macOS`.

Each profile uses one `xcodebuild test` command to build and launch the real app through XCUITest.
Before attachment export or PASS, the runner checks actual xcresult method identities and summary counts against every selected method.
Zero, partial, duplicate, unexpected, skipped, or failed methods fail the gate, regardless of Xcode's exit status.
Each profile retains the tests/summary JSON and `test-verification.json` evidence.
Simulator tests disable code signing; macOS tests use local ad-hoc signatures without a developer account.
The runner disables parallel tests, retains booted simulators, and does not change host permissions.
Xcode limits each test to 120 seconds and destination discovery to 60 seconds.
GNU timeout limits core preparation to 1200 seconds, each build/test command to 480 seconds,
generation and export to 60 seconds, and simulator discovery to 30 seconds.
Each deadline permits ten seconds before a forced stop.

`E2ETests/HomeE2ETests.swift` checks home headings, the first poster, and the sidebar toggle,
then exercises library hierarchy, details without playback, breadcrumbs, search options,
actual refresh status and the absence of unsupported authentication controls.
Simulator captures wait for sidebar-control readiness and check visible glyph pixels, not accessibility visibility alone.
Sidebar and breadcrumb controls use explicit symbols and bounded focus backgrounds without inherited glass rendering.
On macOS, the test checks accessibility values when a static-text label is empty.
The happy-path test captures exactly one successful attachment per showcase flow after its UI assertions:

| Attachment | State |
| --- | --- |
| `home-success` | Home with the first catalog row. |
| `sidebar-success` | Expanded sidebar with library choices; tvOS shows the closed Subtitles disclosure and footer actions. Other wider layouts show subtitle choices. |
| `library-success` | Movies grid with the sidebar collapsed. |
| `search-success` | All libraries, `Bunny` query, default Movies and Shows, All catalogs, Settings closed, and no keyboard. |
| `detail-success` | Big Buck Bunny detail from Movies, collapsed sidebar, top position, info closed, and no playback controls. |

The iPhone and iPad tests submit Return to dismiss the keyboard before the search capture and assert its absence.
These captures alone do not establish a successful suite.
macOS captures only the app window; simulators capture the app screen.
Tests use the real app, native Rust core, and persistent backend with offline adapters, not replacement screens or bundled catalogs.
These smoke checks do not cover every search combination, text size, or device size.
The iPhone, iPad, and Mac tests cover query search, Jellyfin and TMDB provider filters,
detail-return context, Clear query, and no-result status. All profiles assert Settings starts closed
and check the provider information disclosure during the separate Nosferatu detail journey.
`testFreshSearchAndHomeScope` checks fresh-search resets, retained item types, library hierarchy
restoration, and All libraries scope after Home on every platform.
`testHappyPathShowcase` enters the real `Bunny` query through XCUITest on every profile, separate from stateful search regression journeys.
`testSearchPaginationAndPreferencesRoundTrip` checks a multi-page query and preference retention
across app relaunches in the same local settings namespace. `testCoreUnavailableAndRetry` uses a closed
local endpoint and checks honest error/retry behavior without catalog fallback. It then restores the configured healthy endpoint
and relaunches the same app to check Home and server artwork. It never restarts the shared backend or proves in-process recovery.
The tvOS test uses remote directional focus and Select for navigation and options, plus native text entry for the search capture.
`testSidebarRemoteControls` additionally checks every subtitle choice and its collapsed current value,
Refresh, the absence of unsupported authentication controls, and directional focus return without changing the selected destination.
After text entry, the tvOS test dismisses the native keyboard with Menu before the capture.
Inspect narrow layouts and tvOS focus separately; SwiftUI buttons and text fields retain native
keyboard and remote interaction.

macOS XCUITest requires an interactive desktop session and host automation/accessibility approval.
If macOS denies access or displays a permission prompt, report the test as unverified.
Do not change permission databases or approve permissions automatically.

## Artifacts and reuse

Each invocation creates a unique directory under ignored `build/e2e/runs/`.
It contains `generate.log` and a directory for each profile.
Each profile stores `test.log`, `Home.xcresult`, `export.log`, and screenshot attachments.
The runner exports a profile's screenshots only after its entire test suite passes.
Unique run directories prevent stale captures from a previous run.
Inspect the result bundle and exported PNGs separately; build success alone does not establish E2E success.

The root `bash scripts/e2e.sh --update-showcase apple` command supplies a persistent absolute `E2E_APPLE_RUN_DIR`.
The default path is `apps/apple/build/e2e/runs/showcase-XXXXXX/run`; the root runner prints the resolved path.
A caller override requires an absolute, new directory with a parent that exists.
The Apple runner requires that directory not already exist.
The root runner reads that exact run's attachment manifests and requires one non-failure PNG per flow and selected profile.
It preserves the `home-success` attachment contract and also requires `sidebar-success`, `library-success`, `search-success`, and `detail-success`.
Default showcase updates export iOS, iPadOS, and tvOS. They preserve the current macOS images.
Use `E2E_MACOS=1 bash scripts/e2e.sh --update-showcase apple` to require all five current-run captures from all four profiles.
It updates the website only after all selected suites and captures pass.
The root runner removes only its separate temporary screenshot-copy stage on exit; Apple logs and result bundles persist.

The runner reuses the XcodeGen cache and per-scheme DerivedData under `build/e2e/cache/`.
iPhone and iPad tests share `build/e2e/cache/iOS/`; tvOS and macOS use `tvOS/` and `macOS/`.
Each rerun checks source changes and rebuilds the affected targets.
Do not run concurrent invocations against the same scheme cache.

To export attachments again:

```sh
xcrun xcresulttool export attachments --path apps/apple/build/e2e/runs/<RUN>/<PROFILE>/Home.xcresult --output-path apps/apple/build/e2e/runs/<RUN>/<PROFILE>/screenshots
```

The export creates PNGs and a `manifest.json` that maps attachments to tests.
Inspect the screenshot visually after the assertions pass.
Keep generated `Screenroom.xcodeproj`, results, and captures out of source control.
Edit `project.yml` to change targets or schemes; preserve the icon resources.
