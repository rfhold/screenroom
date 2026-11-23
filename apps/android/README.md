# Android clients

Native Compose clients for Android mobile and Android TV. Both activities use the shared `ui` library and an isolated neutral dark theme.

The Rust UniFFI client core supplies Browse, Catalog, Search, User, artwork, and playback controls through the persistent local backend.
See [the client API contract](../../docs/client-api.md). No provider credentials are required.
Catalog RPCs use `screenroom.catalog.v1`; UserService uses `screenroom.user.v1`.
Immutable ClientConfig holds endpoint, local user UUID selector, and timeout. Each scoped read carries explicit source/library scope.
Home rows, library IDs/order/titles, children, complete paginated credits, rights, ratings, and provenance come from the server.
Every artwork record, including provider logos, comes from the server. All bytes pass through the core and decode off the UI thread.
Failed images show placeholders. APKs contain no catalog JSON, media, portraits, or provider branding; app icons and native glyphs remain local.
Compose displays licensed posters, portraits, and character stills where available; other cards use abstract placeholders or initials.
Cast and crew cards preserve snapshot roles. Character stills carry explicit labels.
There is no global attribution or technical-credit footer, footer-only request, or artwork inventory request.
Backend CatalogInfo, artwork credits, provenance, and rights APIs remain available; no provider-credit UI is required.
Portrait cards and item details omit per-item rights, provenance, artwork license, creator, crop, and legal panels.
Backend and core records retain those fields. Per-item source identifiers, external links, ratings, snapshot disclosures, provider status, and people labels remain available.
Compose does not supply fixture-specific identity, legal, or provider notices.

Posters show a top-right 26 dp dark-circle checkmark with a light stroke only when watched state explicitly equals `true`.
Their accessible description includes `Watched`. False and unknown values show no checkmark; progress bars remain independent.
Series and season completion never follows from episode progress.
Episode cards show the series name, a small muted number such as `S05:E09`, and the episode title below.
An explicit season grid uses `E09` only when the episode's parent matches that season.
Unknown episode numbers stay absent. The core supplies episode artwork and the season-grid artwork exception.
Artwork reads share URLs and limit active work to eight requests.
Cancellation detaches individual consumers and preserves reads that other consumers still need.

The sidebar expands within the layout. Mobile uses a fixed top toggle with horizontally scrollable navigation links.
TV uses a 72 dp side rail that expands to 19% of the configured dp screen width, bounded to 180–260 dp.
Its 48 dp navigation rows keep destination selection separate from remote-focus outlines.
Libraries can scroll within the sidebar while Refresh remains below navigation. Playback settings contain subtitle controls.
Left from the Home entry control enters the sidebar. Right returns to the last focused content control and collapses the sidebar without activating a destination.
The sidebar returns to its top whenever it expands or collapses.
Library grids lead through shows, seasons, episodes, and title details. Breadcrumbs and Android Back retain navigation context.
Library, children, and search grids request 100 items initially, then expose Load more and loaded counts.
Continuation pages preserve API order and suppress duplicate stable IDs. Page errors preserve loaded cards and the retry token.
Repeated-token guards, bounded traversal, cancellation, and request identity checks protect continuation state.
The Jellyfin profile page lists sources, eligible profiles, and the current server link for the selected local UUID.
Save and Unlink issue one mutation, invalidate reads, and reread the link. Ambiguous failures do not trigger automatic mutation retries.
Retry reads server state only; unknown link state disables mutation controls. These controls provide no sign-in or operator interface.
Continue Watching failures show a generic explanation while other Home rows remain usable.
Search uses the current library and a 350 ms debounce.
The initially closed Settings panel contains every catalog, type, and method filter.
Search capabilities supply the available sources, types, methods, maximum query length, and semantic description.
Known protocol fields receive UI labels; Compose does not add choices absent from those capabilities.
Title details preserve server rating provider/scale, provider identities/statuses/valid links, runtime notes, and metadata snapshot disclosures.
Unknown vote counts retain numeric scores; explicit zero counts show Unrated. Missing ratings remain distinct from missing metadata dates.
Abstract poster placeholders use server colors, with neutral colors only for missing or invalid palette values.
Compact text breadcrumbs retain search returns within one row.
Detail headers use two columns at 640 dp of available content width; narrower screens stack the 160 × 240 dp poster above the summary.
Wider mobile layouts use 200 × 300 dp posters. TV uses 140 × 210 dp posters, or 280 × 420 pixels on the 1080p profile.
Cast and Crew follow the header at full content width.
People grids wrap across rows with square images and complete names, roles, and character-still labels.
TV uses 90 dp image squares, equivalent to 180 pixels on its 1080p profile.
Keyboard and remote focus remains visible. Touch input does not add keyboard-focus outlines to buttons.
TV entry focus selects content controls; focus scrolls only enough to reveal an offscreen control.
That disclosure includes vote counts, the snapshot date, and the distinction between catalog membership and playback availability.
Search calls the core after the debounce; there is no Kotlin matching algorithm. Title-only mode disables Semantic without automatically restoring it.
At least one type and one method remain selected.
Each immutable search history entry retains its query, catalog, types, and methods across details and Android Back.
Fresh searches reset query, catalog, and methods. Type preferences persist in local settings.

Search settings use validated version-1 SharedPreferences JSON with synchronous `commit` and explicit failure reporting.
Old settings files ignore the obsolete subtitle field. The shared core normalizes saved `tmdb` to `all` and preserves unrelated valid settings.
Library IDs are opaque; literal `movies` or `tv` values imply no media type or navigation behavior.
Refresh invalidates core reads and fetches them again.
The refresh status reports actual catalog read success or failure, not a local counter.
There is no Welcome/logout flow, authentication, synchronization, transcription, or operator UI.
Navigation history is presentation state; local preferences survive app restarts independently of backend state.
Loading, error, and retry states do not block the UI thread. Superseded read jobs are canceled and guarded against stale results.
Native deep links are not implemented.

## Playback

Movie and Episode details expose Play. Both apps use Media3 ExoPlayer for MP4, HLS, and gateway WebVTT subtitles.
The ordinary shared Client controls sessions; the player reads media directly from gateway URLs.
The apps never send playback requests directly to Jellyfin.
The gateway reads the current personal profile's resume position; catalog percentages never supply seek positions.
Playback settings offer gateway audio/subtitle choices, explicit Off, session checks, exact-request retries, and one compatible-transcode fallback.
Unknown personal-state outcomes stop media and block automatic report replay.
Controls include a seek bar, touch input, keyboard focus, Android TV remote buttons, and media play/pause/seek keys.
Catalog history retains bounded entry-owned results, continuation tokens, query/options/scope, scroll positions, and the last focused TV control across detail and player visits.
Refresh and profile/client changes invalidate those reads. Scroll restoration has a finite wait.
Enter fullscreen hides system bars on mobile and TV without replacing the player or session. Back first exits fullscreen.

Exit, Android Back, background, disposal, and client changes stop local media and submit a fixed final position.
Local media stopping does not imply server confirmation. Pending or uncertain cleanup retains its exact stop request in memory for up to one minute.
The player and same-profile catalog offer explicit stop retry and read-only cleanup checks; unknown stops never retry automatically.
New playback waits while same-profile recovery is open. Dispatching a profile save/unlink advances UI ownership immediately: old recovery actions disappear and cannot reattach or block the replacement profile.
Ordinary catalog reloads do not advance that ownership. Profile replacement/client disposal leave the old controller's bounded private cleanup running independently.
After the recovery deadline, the client closes and authority is discarded without claiming confirmation.
The player retains session authority only in memory. Process restoration never resumes playback.
After a background stop, return to details and select Play for a new session.
Picture-in-picture, background playback, downloads, casting, and external-player handoff remain unsupported.
Isolated checks do not establish device, decoder, or live-provider playback acceptance.

## Build

Use JDK 17, Android SDK 37, and the checked-in Gradle wrapper. Preserve the dependency and plugin pins.
Set environment variables instead of changing `local.properties`:

```sh
cd apps/android
export JAVA_HOME=/path/to/jdk-17
export ANDROID_HOME=/path/to/android/sdk
./gradlew --build-cache :mobile:assembleDebug :tv:assembleDebug \
  :ui:testDebugUnitTest \
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
`SCREENROOM_USER_ID` override the endpoint and local user UUID selector. The selector is not authentication.
The selector defaults to empty. Supply the configured installation UUID; no demo-user fallback exists. Provider setup belongs to the Web app, not a native credential form.
`SCREENROOM_E2E_SETTINGS_NAMESPACE` selects a local preference namespace, not a backend user.
Android validates the endpoint before core construction because native sockets bypass Android network security configuration.
Release builds require HTTPS. Their default HTTP endpoint displays a configuration error and never creates a core client.
Debug builds permit HTTP only for the exact hosts `127.0.0.1` and `localhost`; IPv6 loopback is not enabled.
Both build types reject credentials, paths, query strings, fragments, and malformed hosts.
Refresh restarts active search and artwork reads without a navigation, query, or focus reset.
For an approved backend instance, use `adb -s <serial> reverse tcp:50051 tcp:50051` so artwork shares the API origin.
For an alternate backend port, use that same port on both sides of ADB reverse.

## E2E suite

From the repository root:

```sh
export SCREENROOM_BACKEND_ATTACH=1
export SCREENROOM_API_ENDPOINT=http://127.0.0.1:50051
export SCREENROOM_E2E_API_ENDPOINT="$SCREENROOM_API_ENDPOINT"
export SCREENROOM_USER_ID='<enabled local user UUID from this installation>'
export SCREENROOM_BACKEND_DESCRIPTOR="$PWD/build/backend-runtime/screenroom-e2e/descriptor.json"
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
| `happy-path` | Real Home, sidebar, discovered library, title search and discovered movie details; TV center activation and detail geometry | Five per profile |
| `retry` | Actual closed-loopback RPC error, real Retry while unavailable, then a real Activity relaunch with the configured healthy endpoint and Home/artwork recovery | None |
| `regression` | Source-driven core pagination, same-origin artwork and native decoding, search-type persistence, real refresh and deadline/save-failure recovery | None |
| `all` | Each category once, with separate local settings namespaces | Only the happy-path set |

Retry does not stop or restart the shared backend. It does not prove in-place recovery after a live server restart.
The test reserves and closes an emulator-local ephemeral port, then launches the real Activity against that unavailable endpoint.
It verifies honest errors and no fixture/artwork fallback before and after Retry.
Recovery uses another real Activity launch against the configured endpoint, not injected UI state.
To extend the suite, add a shared assertion and a JUnit method to both profile test classes.
Add the category/method pair to the early category map in `scripts/e2e.sh`, including the `all` list.
Each category receives an isolated run/profile/category local settings namespace.

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
| `SCREENROOM_API_ENDPOINT` | Required caller-owned backend origin with an explicit loopback port. |

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
The suite requires the shared backend descriptor, attach flag, endpoint, and explicit local user UUID before instrumentation.
It verifies semantic/artwork readiness and live instance identity; it never starts a separate API server.
The endpoint must include an explicit port. Each verified emulator receives a matching `adb reverse` mapping.
Existing matching mappings are reused; conflicts are rejected. Cleanup removes only mappings created by the suite.
Instrumentation receives the endpoint, user UUID, and unique run-prefixed local settings namespaces before Activity launch.
Contract clients use the same persistent backend and explicit scopes.
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
Shared assertions discover libraries and a movie with published artwork through the public core. Real-data journeys fail with an explicit prerequisite message when none exists; they do not seed a catalog or assume titles, library IDs, counts, notices, or branding. Hierarchy regression also requires an indexed series with a season and episode. Unit tests independently cover episode/season poster fallback, nullable watched state, profile-generation isolation, pagination/cancellation, presentation boundaries, and native playback ownership/retries.
Only the happy-path test writes the five showcase flows with `UiAutomation`.
The regression test writes no PNGs. Captures follow Compose and accessibility idle; they use the discovered library and movie title, not a fixed result count. Detail assertions retain poster aspect ratio, responsive placement, and the Play action. These checks do not activate playback. Device E2E execution is separate from local unit and instrumentation compilation checks and requires explicit device authority.

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
