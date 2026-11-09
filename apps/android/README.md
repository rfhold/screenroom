# Android clients

Native Compose clients for Android mobile and Android TV. Both activities use the shared `ui` library and an isolated neutral dark theme.

The Rust UniFFI client core supplies Browse, Catalog, Search, and Preferences through the local mock API.
See [the client API contract](../../docs/client-api.md). No provider credentials are required.
Home rows, library IDs/order/titles, children, complete paginated credits, rights, ratings, and provenance come from the server.
Every artwork record, including provider logos, comes from the server. All bytes pass through the core and decode off the UI thread.
Failed images show placeholders. APKs contain no catalog JSON, media, portraits, or provider branding; app icons and native glyphs remain local.
Compose displays licensed posters, portraits, and character stills where available; other cards use abstract placeholders or initials.
Cast and crew cards preserve snapshot roles. Character stills carry explicit labels.
The global Data & artwork credits disclosure uses `GetCatalogInfo` for the complete artwork inventory, retrieval timestamp, and ordered notices.
Credits include unvisited portraits, each artwork creator/license/source/download, server logos, item provenance notes, and title/metadata rights.
Compose does not supply fixture-specific identity, legal, or provider notices.

The sidebar expands within the layout. Mobile uses a fixed top toggle with horizontally scrollable navigation links.
TV uses a 72 dp side rail that expands to 19% of the configured dp screen width, bounded to 180–260 dp.
Its 48 dp navigation rows keep destination selection separate from remote-focus outlines.
Libraries can scroll within the sidebar while Subtitles and Refresh remain pinned below navigation.
Subtitles starts closed and shows its current value; Select opens a remote-focusable three-choice popup.
Left from the Home entry control enters the sidebar. Right returns to the last focused content control and collapses the sidebar without activating a destination.
The sidebar returns to its top whenever it expands or collapses.
Library grids lead through shows, seasons, episodes, and title details. Breadcrumbs and Android Back retain navigation context.
Search uses the current library and a 350 ms debounce.
The initially closed Settings panel contains every catalog, type, and method filter.
Search capabilities supply the available sources, types, methods, maximum query length, and semantic description.
Known protocol fields receive UI labels; Compose does not add choices absent from those capabilities.
Title details preserve server rating provider/scale, provider identities/statuses/valid links, runtime notes, metadata dates, and rights.
Unknown vote counts retain numeric scores; explicit zero counts show Unrated. Missing ratings remain distinct from missing metadata dates.
Abstract poster placeholders use server colors, with neutral colors only for missing or invalid palette values.
Compact text breadcrumbs retain search returns within one row.
Detail headers use two columns at 640 dp of available content width; narrower screens stack the 160 × 240 dp poster above the summary.
Wider mobile layouts use 200 × 300 dp posters. TV uses 140 × 210 dp posters, or 280 × 420 pixels on the 1080p profile.
Cast, Crew, Title rights, and credits follow the header at full content width.
People grids wrap across rows with square images and complete names, roles, and character-still labels.
TV uses 90 dp image squares, equivalent to 180 pixels on its 1080p profile.
Keyboard and remote focus remains visible. Touch input does not add keyboard-focus outlines to buttons.
TV entry focus selects content controls; focus scrolls only enough to reveal an offscreen control.
That disclosure includes vote counts, the snapshot date, and the distinction between catalog membership and playback availability.
Search calls the core after the debounce; there is no Kotlin matching algorithm. Title-only mode disables Semantic without automatically restoring it.
At least one type and one method remain selected.
Each immutable search history entry retains its query, catalog, types, and methods across details and Android Back.
Fresh searches reset query, catalog, and methods. Type preferences persist in the mock profile.

Subtitles and search types use Get/UpdatePreferences. Refresh invalidates core reads and fetches them again.
The refresh status reports actual catalog read success or failure, not a local counter.
There is no Welcome/logout flow, authentication, synchronization, transcription, or playback.
Navigation history is presentation state; preferences persist for the lifetime of the mock server process.
Loading, error, and retry states do not block the UI thread. Superseded read jobs are canceled and guarded against stale results.
Native deep links are not implemented.

## Build

Use JDK 17, Android SDK 37, and the checked-in Gradle wrapper. Preserve the dependency and plugin pins.
Set environment variables instead of changing `local.properties`:

```sh
cd apps/android
export JAVA_HOME=/path/to/jdk-17
export ANDROID_HOME=/path/to/android/sdk
./gradlew --build-cache :mobile:assembleDebug :tv:assembleDebug \
  :mobile:lintDebug :tv:lintDebug \
  :mobile:assembleDebugAndroidTest :tv:assembleDebugAndroidTest
```

On Homebrew macOS, JDK 17 commonly resides at `/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home`.
The SDK commonly resides at `$HOME/Library/Android/sdk`.
The UI module consumes generated Kotlin bindings and arm64-v8a/x86_64 JNI libraries from `target/client-core/`.
Its single preparation task invokes `scripts/build-core.sh`, which hashes core inputs and runs the root Android build helper only when stale.
The cache key includes the resolved Rust compiler/toolchain, NDK paths and versions, SDK path, and `SCREENROOM_CORE_PROFILE`.
Install the root helper's Rust targets and Android NDK r29 before the first build.
Run `bash scripts/test-build-core-cache.sh` to check cache invalidation with mock compilers and no native cross-build.
JNA 5.17.0 is packaged as an AAR.

Debug apps default to `http://127.0.0.1:50051`. Launch intent string extras `SCREENROOM_API_ENDPOINT` and
`SCREENROOM_MOCK_PROFILE` override the endpoint and preference profile.
Android validates the endpoint before core construction because native sockets bypass Android network security configuration.
Release builds require HTTPS. Their default HTTP endpoint displays a configuration error and never creates a core client.
Debug builds permit HTTP only for the exact hosts `127.0.0.1` and `localhost`; IPv6 loopback is not enabled.
Both build types reject credentials, paths, query strings, fragments, and malformed hosts.
Refresh restarts active search and artwork reads without a navigation, query, or focus reset.
For a manually started mock server, use `adb -s <serial> reverse tcp:50051 tcp:50051` so returned artwork URLs share the same origin.

## E2E suite

From the repository root:

```sh
bash apps/android/scripts/e2e.sh          # both profiles
bash apps/android/scripts/e2e.sh mobile   # mobile only
bash apps/android/scripts/e2e.sh tv       # TV only
E2E_CATEGORY=happy-path bash apps/android/scripts/e2e.sh
E2E_CATEGORY=retry bash apps/android/scripts/e2e.sh
E2E_CATEGORY=regression bash apps/android/scripts/e2e.sh
```

`E2E_CATEGORY` accepts `happy-path`, `retry`, `regression`, or `all` (the standalone default).
Unknown and explicitly empty categories fail before screenshot cleanup, server startup, or device work.
Only an unset category selects the standalone default. Run `bash apps/android/scripts/test-e2e-categories.sh`
from the repository root to verify rejection and preservation of ten sentinel captures in an isolated fixture, without devices.
The root runner selects its own category defaults; screenshot exports use `happy-path`.

| Category | Coverage | Screenshots |
| --- | --- | --- |
| `happy-path` | Real Home, default Off sidebar, Movies, default All-catalog Bunny search, and Big Buck Bunny details from Movies; TV remote focus, data/artwork readiness, and layout geometry | Five per profile |
| `retry` | Actual closed-loopback RPC error, real Retry while unavailable, then a real Activity relaunch with the configured healthy endpoint and Home/artwork recovery | None |
| `regression` | Hierarchy, ratings, source/method/type guards, history, subtitle choices, real refresh, absent fake auth, core pagination/preferences contracts, endpoint rejection, and CatalogInfo/logo PNG decoding | None |
| `all` | Each category once, with separate preference profiles | Only the happy-path set |

Retry does not stop or restart the caller's mock server. It does not prove in-place recovery after a live server restart.
The test reserves and closes an emulator-local ephemeral port, then launches the real Activity against that unavailable endpoint.
It verifies honest errors and no fixture/artwork fallback before and after Retry.
Recovery uses another real Activity launch against the configured endpoint, not injected UI state.
To extend the suite, add a shared assertion and a JUnit method to both profile test classes.
Add the category/method pair to the early category map in `scripts/e2e.sh`, including the `all` list.
Each category receives an isolated run/profile/category preference namespace.

The Bash suite requires `file`, JDK 17, the Android SDK, coreutils `timeout`, and prepared AVDs.
It never installs SDK packages or creates AVDs.
It verifies explicit emulator serials, builds APKs incrementally with the Gradle daemon and build cache, then runs tests.
It rejects physical devices, incorrect AVD names, and mismatched TV features.

| Environment variable | Default |
| --- | --- |
| `JAVA_HOME` | `/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home` |
| `ANDROID_HOME` | `ANDROID_SDK_ROOT`, or `$HOME/Library/Android/sdk` |
| `ANDROID_AVD_HOME` | `apps/android/build/avd` when present; otherwise the emulator default |
| `E2E_ANDROID_MOBILE_SERIAL` | `emulator-5554` |
| `E2E_ANDROID_TV_SERIAL` | `emulator-5556` |
| `E2E_ANDROID_MOBILE_AVD` | `screenroom_home_mobile` |
| `E2E_ANDROID_TV_AVD` | `screenroom_home_tv` |
| `SCREENROOM_API_ENDPOINT` | Owned mock server at `http://127.0.0.1:50062` |

From `apps/android`, prepare ARM64 Android 36 emulators under the ignored `build/` directory:

```sh
export ANDROID_AVD_HOME="$PWD/build/avd"
mkdir -p "$ANDROID_AVD_HOME"
"$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" \
  'system-images;android-36;google_apis;arm64-v8a' \
  'system-images;android-36;android-tv;arm64-v8a'
echo no | "$ANDROID_HOME/cmdline-tools/latest/bin/avdmanager" create avd \
  --name screenroom_home_mobile --package 'system-images;android-36;google_apis;arm64-v8a' \
  --device pixel_7 --path "$ANDROID_AVD_HOME/screenroom_home_mobile.avd"
echo no | "$ANDROID_HOME/cmdline-tools/latest/bin/avdmanager" create avd \
  --name screenroom_home_tv --package 'system-images;android-36;android-tv;arm64-v8a' \
  --device tv_1080p --path "$ANDROID_AVD_HOME/screenroom_home_tv.avd"
```

For Intel hosts, select supported x86_64 system images and compatible profiles instead.
The runtime API 36 profiles do not change the compile or target SDK 37 pins.

The suite reuses a verified emulator or boots the configured AVD headlessly, with snapshots allowed.
Boot has a 180-second deadline and instrumentation has a 300-second deadline; each APK install has a 120-second deadline.
The build has a 600-second deadline; screenshot export has a 30-second deadline.
The suite stops its own emulator process if startup or device verification fails.
After successful startup verification, the emulator stays available even if a test fails.
Other devices remain unchanged.
Without an endpoint override, the suite starts the root mock-server helper, waits for `/health`, logs it under `build/e2e/`, and stops only its own process on exit.
An external endpoint must include an explicit port. Each verified emulator receives a matching `adb reverse` mapping.
Existing matching mappings are reused; conflicts are rejected. Cleanup removes only mappings created by the suite.
Instrumentation receives the endpoint and unique run-prefixed mobile/TV profiles before Activity launch.
Contract clients use separate profiles under the same prefix, including with caller-owned mock servers.
`SCREENROOM_E2E_PROFILE_PREFIX` overrides the prefix and accepts 1–90 ASCII letters, digits, underscores, or hyphens.

The suite clears all five selected-profile host screenshots before the build.
It also clears private screenshots on each verified emulator before the build, when the app already exists.
The suite installs debug and test APKs, wakes the display, dismisses the keyguard, and clears private screenshots again.
It exports screenshots to temporary files only after the complete profile test passes.
It checks every selected-profile PNG signature and positive image dimensions before it publishes any final screenshot.
On failure, it removes final and temporary screenshots for all selected profiles.
It also attempts to clear private screenshots on verified emulators; unreachable devices can retain private files.
To stop an emulator explicitly:

```sh
"$ANDROID_HOME/platform-tools/adb" -s emulator-5554 emu kill
"$ANDROID_HOME/platform-tools/adb" -s emulator-5556 emu kill
```

Use the selected serial if you override a default.
Gradle retains `apps/android/.gradle`, module `build/` directories, and `$GRADLE_USER_HOME` (default `~/.gradle`) for fast reruns.
AVDs retain their data and snapshots under `ANDROID_AVD_HOME`.

Each test launches the real `MainActivity` through an intent-aware `AndroidComposeTestRule`.
Shared assertions cover all Home rows, sidebar controls, TV-show hierarchy, episode details, breadcrumbs, scoped search, and search return state.
They also cover inline sidebar expansion, title-only constraints, the last-type and last-method guards, clear query, no matches, absence of mock authentication controls, and completed server refreshes.
The TV profile additionally checks D-pad sidebar entry and focus return, persistent Home selection, subtitle choice activation and current values, visible Refresh activation, and absence of mock authentication controls.
It also checks the sidebar width fraction, D-pad card traversal, automatic horizontal scroll, center activation, and Android Back.
Only the happy-path test writes the five showcase flows with `UiAutomation`.
The regression test retains the unrelated navigation, filter, preference, and capture-state assertions, but writes no PNGs.
Each capture follows actual data/artwork readiness, assertions, Compose idle, and accessibility idle.
The flows show Home, the expanded sidebar, the Movies grid, populated All libraries search, and movie details without playback.
Search uses the query `Bunny` and verifies one result, Big Buck Bunny, on both profiles.
The test transfers focus from the query to Settings and dismisses the software keyboard.
Mobile then returns to touch mode. TV retains visible Settings focus.
The test checks keyboard absence, query, count, and result visibility both before and after capture.
Every capture checks top scroll positions. Details originate from Movies, not Search, with metadata information closed.
Detail geometry assertions cover the breakpoint, poster aspect ratio, breadcrumb height, and full-width Cast placement below the header.
The TV search capture shows the result count and the upper portion of the result poster within the viewport.
Both profiles check nested search histories, distinct query and method restoration, Android Back, and Search results returns.

The script exports the PNG from the debug app's private test output through serial-specific `adb exec-out run-as`.
Outputs relative to `apps/android`:

- `build/e2e/mobile-{home,sidebar,library,search,detail}.png`
- `build/e2e/tv-{home,sidebar,library,search,detail}.png`
- `build/e2e/*-instrumentation.txt`
- `build/e2e/{mobile,tv}-{happy-path,retry,regression}-instrumentation.txt` for selected categories
- `build/e2e/*-emulator.log` when the suite starts an emulator
- `build/e2e/build.log`

These tests do not cover rotation, process restoration, accessibility services, every keyboard interaction, or pixel-golden comparison.
System bars, fonts, and display sizes depend on the emulator image. Screenshots are evidence, not committed golden files.
