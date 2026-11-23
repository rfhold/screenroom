# Apple apps

The iOS, iPadOS, macOS, and tvOS apps share native SwiftUI views and an isolated dark theme.
`Sources/Home/Catalog.swift` adapts Rust UniFFI domain records for presentation.
The Rust client core owns Browse, Catalog, Search, User, and artwork requests to the
[persistent local backend](../../docs/client-api.md). Home rows, library membership, hierarchy, credits,
and search results come from that API. Item grids show the first page, with up to 100 items, then offer Load more.
Continuation errors retain current items and the retry token. Appends preserve API order and deduplicate stable IDs.
Preferences use version-1 Foundation atomic JSON in Application Support, not a server preference service.
Legacy UserDefaults migration runs only when no settings file exists; persistence errors retain choices and expose a save-retry action.
There is no global attribution or technical-credit footer, footer-only request, or artwork inventory request.
Backend CatalogInfo, artwork credits, provenance, and rights APIs remain available; no provider-credit UI is required.
Item details and people omit per-item rights, provenance, artwork creators, licenses, and crop notices.
All visible artwork bytes load through the core with explicit loading and failure states.
An eight-request FIFO queue shares concurrent URL requests and retains a bounded byte cache. Cancellation releases queued or active work.
The app contains no bundled catalog JSON, media, portraits, or provider branding. Native app icons and navigation glyphs remain local.
Movie and episode details offer native AVPlayer/AVKit playback through the shared client's public playback API.
The apps have no authentication or direct native media-provider connection.
Server-saved Jellyfin profile mappings select progress snapshots for the local user selector; they do not establish sign-in.

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
The selector defaults to empty. Use the UUID from the configured installation; no demo-user fallback exists. Web setup owns provider credentials and initial connection configuration. Native clients consume the public core only.
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
Detail pages show server artwork, cast/crew roles and portraits, provider references, and rating snapshots.
Provider identity, status, links, rating scale, vote counts, palette, and metadata dates come from server records.
Missing vote counts remain distinct from zero votes. Missing colors use a neutral placeholder palette.
The detail header uses two columns at 640 logical points of available content width.
Narrow layouts stack a 160×240 poster above the summary; tablet and desktop posters use 200×300, and TV posters use 240×360.
The provider row precedes the synopsis. Cast and Crew span the content width below the header.
Adaptive people grids preserve full names, roles, initials, and character-still labels.
Episode cards show the series name and muted `S05:E09`, with the episode title below.
Explicit season children show `E09` instead. Missing numbering context uses Episode without invented numbers.
An explicit true watched state adds a poster check; playback progress remains separate.
Back and breadcrumb buttons return to the originating library, home, or search context.
Bounded in-memory restoration retains scroll targets and focus for 64 library/query/filter/hierarchy contexts.
Entry-owned snapshots retain result prefixes, pagination tokens, continuation errors, and search context for up to 16 entries and 10,000 records.
Back restores those snapshots before scroll and focus; refresh, profile changes, source changes, and new clients discard stale snapshots.

The Jellyfin profile destination lists server-provided sources and profiles, the current mapping, Save, and Unlink.
After one write, the app invalidates reads and checks server state. Retry only reloads that state; it never repeats an ambiguous write.
Unavailable linked profiles remain distinct from an unlinked selector. Confirmed or reconciled changes refresh Home reads.
Continue Watching errors show a sanitized row message and Retry; other rows remain available.

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
Catalog search has no subtitle preference. Playback exposes gateway track choices and Subtitles Off.
Old settings files ignore the obsolete catalog subtitle field.
The shared core normalizes a saved `tmdb` provider preference to `all` and preserves unrelated valid settings.
Library IDs are opaque and imply no icon or media type, even for literal `movies` or `tv` values.
Refresh invalidates core reads and reloads active catalog, search, and artwork tasks.
The status reports success or failure only after the active catalog/search request completes.
The app has no Log out, Welcome, or Continue authentication flow.
On tvOS, sidebar destinations use uniform 56-point rows with separate selected and focused
appearances. Refresh remains below the library navigation, which can scroll. Expansion pushes the content.

## Native playback

Play creates an in-memory session with the current source/library scope and live Jellyfin resume policy.
Restored catalog navigation never starts playback.
AVKit receives gateway media URLs, not Jellyfin URLs or provider credentials.
The app advertises H.264/AAC MP4, native HLS, and WebVTT support.
Plain WebVTT captions use gateway bytes and the logical media timeline.
Custom WebVTT styles and cue placement are not reproduced.
Gateway track changes support audio, subtitles, burn-in choices, and explicit Off.
Unavailable tracks remain disabled. A compatible transport request remains explicit.

Controls expose Play/Pause, backward and forward seek, mute, volume, track choices, session checks, and Stop.
iOS and macOS also expose an absolute-position scrubber and fullscreen controls.
iOS uses an over-fullscreen presentation; macOS uses native window fullscreen; tvOS already occupies the screen.
The tvOS remote Play/Pause command uses the same report-aware playback toggle as the onscreen control.
Presentation transitions preserve the player, session, and event sequence without new playback requests.
The app applies transport offsets once; a seek before the transport origin requests a new gateway timeline.
Events preserve sequence order even for backward seeks.
Active sessions send heartbeats every 15 seconds after the first Play.
An ambiguous personal report blocks later personal reports; a read-only session check never repeats a start.
Preparation, transport, and Stop retries retain the exact request body and mutation UUID in memory.
Exit, unmount, profile change, or an inactive/background scene transition detaches media and freezes the final Stop position.
Foreground activation does not resume the stopped owner; return to details and press Play for a new session.
Fullscreen presentation alone preserves the session; real scene deactivation still stops playback.
The app offers no background playback or Picture in Picture.
Live provider acceptance requires separate device tests.

After core preparation, run isolated playback lifecycle tests without a backend or GUI:

```sh
bash apps/apple/scripts/test-native-playback.sh
SCREENROOM_CORE_PREPARED=1 SCREENROOM_STATE_LOCAL_TESTS_ONLY=1 bash apps/apple/scripts/test-catalog-state.sh
```

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
detail transitions, nullable vote counts, variant provider/scale/palette/capability records, and one-item API pages.
The real-source headless checks traverse discovered libraries and decode published artwork natively when present. Empty catalogs require no branding or notices. `SCREENROOM_STATE_LOCAL_TESTS_ONLY=1` runs the isolated state/persistence checks without server requests; it does not claim real-source integration coverage.
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
| `happy-path` | `testHappyPathShowcase`: five flows using a discovered library/item and one-item pages. Requires indexed real content. | Five per profile. |
| `retry` | `testCoreUnavailableAndRetry`: unavailable endpoint, failed Retry without fallback, then healthy-endpoint app relaunch. | None. |
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

`E2ETests/HomeE2ETests.swift` checks Home, sidebar geometry, discovered library/item navigation, details without playback, breadcrumbs and title search without fixed counts. Real-data journeys fail explicitly if their library/item prerequisites are absent. Hierarchy regression also requires an indexed series with a season and episode. Isolated state tests retain cancellation, pagination, nullable ratings/watched state, profile-generation guards and context replacement coverage.
Sidebar and breadcrumb controls use explicit symbols and bounded focus backgrounds without inherited glass rendering.
On macOS, the test checks accessibility values when a static-text label is empty.
The happy-path test captures exactly one successful attachment per showcase flow after its UI assertions:

| Attachment | State |
| --- | --- |
| `home-success` | Home with the first catalog row. |
| `sidebar-success` | Expanded sidebar with library choices and Refresh, without subtitle controls or attribution footers. |
| `library-success` | Discovered library with the sidebar collapsed. |
| `search-success` | Title search for the discovered item within that library. |
| `detail-success` | Discovered item details without playback controls. |

The iPhone and iPad tests submit Return to dismiss the keyboard before the search capture and assert its absence.
These captures alone do not establish a successful suite.
macOS captures only the app window; simulators capture the app screen.
Tests use the real app, native Rust core, and configured persistent backend, not replacement screens or bundled catalogs.
These smoke checks do not cover every search combination, text size, or device size.
The journeys use titles and IDs observed in the real UI rather than product demo records. `testFreshSearchAndHomeScope` checks All libraries scope after Home.
`testSearchPaginationAndPreferencesRoundTrip` checks absent subtitle controls across relaunches, Refresh, and a real-data navigation journey.
Local state tests cover settings persistence separately. `testCoreUnavailableAndRetry` uses a closed
local endpoint and checks honest error/retry behavior without catalog fallback. It then restores the configured healthy endpoint
and relaunches the same app to check Home and server artwork. It never restarts the shared backend or proves in-process recovery.
The tvOS test uses remote directional focus and Select for navigation and options, plus native text entry for the search capture.
`testSidebarRemoteControls` additionally checks Refresh, absent subtitle controls, and directional focus return without a destination change.
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
