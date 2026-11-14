# Screenroom for Linux

A native GTK4/libadwaita catalog interface, independent of the root Rust workspace.
The Rust client core retrieves configured real catalog data, search results, and artwork through the persistent backend.
The shell has no media files, authentication, subtitle engine, provider credentials, direct provider HTTP, or playback support.
The [wire contract](../../docs/client-api.md) places catalog RPCs in `screenroom.catalog.v1` and UserService in `screenroom.user.v1`.
Immutable ClientConfig holds endpoint, local user UUID selector, and timeout. Scoped requests preserve source/library context through details and continuation pages.

## Interface

- Home contains server-defined poster rows; mapped local users receive actual provider progress or explicit read errors.
- Row failures show a sanitized message and Retry row; other rows remain usable.
- The expanded sidebar offers server-saved Jellyfin profile choices, current state, Save and Unlink. These controls do not provide sign-in or credentials.
- An uncertain profile change requires a server read. Retry never repeats a write; confirmed or reconciled changes refresh Home.
- Profile tasks end when the panel departs. Later reads reconcile uncertain writes; profile reads never invalidate the catalog client's requests.
- The sidebar expands within the layout and stays available across Home, libraries, search, and details.
- Libraries contain responsive poster grids; series open seasons, and seasons open episodes.
- Title details place a 200×300 poster beside the summary, provider row and synopsis, followed by full-width Cast and Crew sections.
- Below 640 logical content pixels, the header stacks with a 160×240 poster. There are no Play or Resume controls.
- General episode cards show the series, a muted season/episode number, and the episode title. Season grids show the episode number and title.
- People use adaptive grids with 140×140 portraits or honest placeholders, wrapping names and roles, and labels identifying character stills.
- The provider row uses server identities, statuses, URLs, rating provider and scale. Its native disclosure shows vote counts, snapshot dates, and provider references; unknown votes, zero votes, and absent ratings remain distinct.
- Clickable breadcrumbs return to libraries, series, seasons, or search results.
- Search preserves its scope, query, and options across detail navigation.
- Search waits 350 milliseconds after query edits and supports clear, blank, and no-results states.
- Search types start from local settings. At least one supported type stays enabled.
- Server capabilities determine the source, content-type and search-method choices, query limit and Semantic description. Controls live inside Settings, closed initially.
- Inline settings offer PostgreSQL full-text search, title similarity, and exact vector Semantic search with explicit lexical degradation.
- Title only disables Semantic. Its later removal does not restore Semantic automatically.
- Search settings persist in validated version-1 private XDG JSON through atomic file replacement.
- Refresh aborts prior loads and searches, invalidates core reads, and reloads server data. Loading, completion, and errors reflect actual reads; there is no mock refresh counter or authentication flow.

The app uses native GTK widgets, server artwork, and Cairo placeholders without a web view.
Its app-scoped CSS matches the neutral dark web design; the app requests libadwaita's dark color scheme.
Navigation state resets when the process exits. Device-local preferences survive app and backend restarts independently.
The default settings path is `$XDG_CONFIG_HOME/screenroom/settings.json`, or `$HOME/.config/screenroom/settings.json`.
`SCREENROOM_SETTINGS_PATH` overrides the path; `SCREENROOM_E2E_SETTINGS_NAMESPACE` isolates test files.
The store reports read and write failures and preserves current session choices.
Old settings files ignore the obsolete subtitle field; no subtitle preference or controls exist.
The shared core normalizes saved `tmdb` to `all` and preserves unrelated valid settings.

`src/catalog.rs` maps core records into thin UI records. The app does not embed catalog JSON or title artwork.
All catalog artwork, including branding, comes from core-loaded server bytes. There are no local catalog, provenance, or artwork bundles; native application icons, CSS, system glyphs and generic placeholders remain local.
The core owns all RPC and artwork HTTP requests; no API key participates.
A two-thread Tokio runtime executes requests and raster image decode outside GTK's main thread.
GTK's main context publishes metadata before artwork. A serial, bounded artwork task updates native pictures as bytes arrive.
Append and watch-state updates retain textures for stable artwork URLs.
The pixel cache caps at 256 entries and 96 MiB; the texture cache caps at 512 entries and 32 MiB.
Live pictures retain their textures independently of cache eviction.
Request generations reject stale results; query edits abort superseded requests. View destruction cancels core requests.
Initial load, error, retry, empty results, and missing-image states never substitute embedded catalog data.

No global attribution or technical-credit footer, footer-only request, or artwork inventory appears.
Backend CatalogInfo, artwork credits, provenance, and rights APIs remain available; no provider-credit UI is required.
Per-item title, metadata and artwork rights, creator credits and provenance notes do not appear in the interface.
Ratings, votes, snapshot disclosure, provider references, people roles and character-still labels remain visible.
The server supplies stable artwork references, regardless of name variations.

## Native run

Use an edition-2024 Rust toolchain, `pkg-config`, GTK 4.10+ development libraries, and libadwaita development libraries.
The Docker build pins Rust 1.98.1 and Debian 13 (Trixie).
On Debian 13, the development packages are `libgtk-4-dev`, `libadwaita-1-dev`, and `pkg-config`.

From the repository root:

```sh
bash scripts/backend.sh up
bash scripts/backend.sh wait
```

These lifecycle commands require explicit approval and the [backend prerequisites](../../docs/backend.md#persistent-local-instance).
In another terminal:

```sh
cargo run --manifest-path apps/linux/Cargo.toml --locked
cargo fmt --manifest-path apps/linux/Cargo.toml --all -- --check
cargo check --manifest-path apps/linux/Cargo.toml --locked
cargo test --manifest-path apps/linux/Cargo.toml --locked --bin screenroom-linux
```

The native run needs a desktop session and the persistent backend.
`SCREENROOM_API_ENDPOINT` defaults to `http://127.0.0.1:50051`; endpoint aliases must equal it when set.
Those aliases are `SCREENROOM_ENDPOINT` and `SCREENROOM_E2E_API_ENDPOINT`.
`SCREENROOM_USER_ID` selects an enabled local UUID, not an account credential or authentication.
The selector defaults to empty; use the configured installation UUID. Web setup owns provider credentials and initial connection configuration. The native shell consumes the public core only.
The core uses a three-second request timeout.
`src/main.rs` owns application setup; `src/home.rs` owns navigation, the sidebar, grids, and details.
`src/backend.rs` loads core metadata pages and decodes artwork.
Library IDs, titles, order, Home rows and membership come from server responses, not local media-kind filters. Navigation retains stable library and item IDs across reordered records.
Library IDs are opaque; literal `movies` and `tv` values imply no special behavior.
Library, child and search grids publish the first 100 results, then offer Load more and a loaded count.
Append preserves API order and stable IDs. A continuation failure retains visible entries and its token for Retry.
Context changes cancel obsolete requests. Repeated tokens, page limits and operation deadlines bound further reads.
Metadata choices and credits still consume their continuations before display.
`src/catalog.rs` maps records; `src/state.rs` owns stable-ID navigation and search request state.
`src/search.rs`, `src/poster.rs`, and `src/theme.css` isolate search controls, poster art, and the app theme.

## Headless Wayland E2E tests

Use Docker with a Linux daemon; Docker Desktop works without host display access.
The first build downloads Debian packages, Rust dependencies, and base images.

```sh
bash apps/linux/tests/e2e/run.sh
E2E_CATEGORY=happy-path bash apps/linux/tests/e2e/run.sh apps/linux/build/e2e-happy
E2E_CATEGORY=retry bash apps/linux/tests/e2e/run.sh apps/linux/build/e2e-retry
E2E_CATEGORY=regression bash apps/linux/tests/e2e/run.sh apps/linux/build/e2e-regression
```

The script builds `screenroom-linux-e2e:local` and runs native Cargo checks, unit tests, and the app build.
Its build context uses the repository root for path dependencies; runtime clients contain no fixture API server.
The Dockerfile-specific allowlist excludes unrelated applications, local builds, and repository metadata.
The runtime image contains the app, test binary, byte-only origin forwarder, helper scripts, and system runtime packages.
The runner requires the shared backend descriptor and verifies readiness and instance identity before app startup.
It joins the descriptor's Compose client network and forwards loopback bytes to the same Envoy edge.
It never starts or stops an API server or substitutes fixture responses.
It compiles the integration test with `cargo test --locked --features e2e --test e2e --no-run`.
BuildKit retains Cargo registry, Git, and target caches between builds.
Source-only edits reuse compiled dependencies; test-only edits do not rebuild the app.
The runtime executes Cargo's compiled integration test binary with standard Rust test discovery and output, without Cargo or a Rust toolchain.
The `e2e` feature gates this test target; ordinary checks do not start a display.
WayDriver and its Mutter backend crates are development dependencies pinned to 0.3.10.
The image uses a non-root Mutter compositor, a headless output, Mesa software rendering, and GTK's Cairo renderer.
The app requires the Wayland backend; Mutter disables X11.
No X11 server, Xvfb, GPU device, privileged mode, host mount, or host display socket participates.
The container uses the shared client network, not `--network none`; private backend ports remain outside that network.
It drops all capabilities and limits memory and process count.

`E2E_CATEGORY` selects `happy-path`, `retry`, `regression`, or `all` (the standalone default).
Unknown categories and conflicting legacy selectors fail before output cleanup or Docker access.
Every scenario uses WayDriver `Session` and XPath locators over AT-SPI on a private D-Bus session.
`happy-path` runs one short native journey through the five canonical capture states, requiring real catalog, search, and artwork readiness.
`retry` runs unavailable-server recovery and artwork-stream fault checks against the configured backend.
`regression` runs two independent scenarios: catalog/preferences and search/history.
Real-data journeys discover a library and movie with published artwork through the public core and fail explicitly when that prerequisite is absent. They assert server-defined Home membership and watched/progress labels, native artwork decoding, settings persistence/save failure recovery, pending search and real Refresh. Unit tests retain profile-generation, episode artwork, pagination/cache and explicit watched-state boundaries. No fixed title, source/library ID or result count is required. Each locator has a ten-second deadline; the runner bounds the test process.
WayDriver manages Mutter, PipeWire, WirePlumber, application startup, and subprocess cleanup.
Its Mutter backend captures the compositor output through PipeWire.
Only `happy-path` captures the five asserted UI states during one journey and lets GTK paint before each capture. Regression scroll assertions compare transient frames but do not publish PNGs. Navigation uses native accessibility actions, alongside pointer scrolling, keyboard input and direct widget clicks; it does not inject application state.
It decodes every PNG and requires 1280×800 pixels with more than eight RGB colors.
It captures the initial Home before any status appears.
It holds all five captures in memory and writes them only after every assertion and session shutdown succeeds.
A 75-second container command timeout bounds each scenario, with a five-second forced-stop grace period. `all` runs four separate container invocations, not one combined timeout.
The script removes the container on exit.

Artifacts default to ignored `apps/linux/build/e2e/`:

| Artifact | Purpose |
| --- | --- |
| `home.png` | Initial All Home with poster rows, collapsed sidebar and no status. |
| `sidebar.png` | Home with the expanded sidebar and Refresh, without subtitle controls or attribution footers. |
| `library.png` | Discovered real library with the sidebar collapsed. |
| `search.png` | Title search for the discovered movie within its library. |
| `detail.png` | Discovered movie details without playback. |
| `<scenario>-accessibility.xml` | Application accessibility tree saved when a scenario reaches cleanup. |
| `e2e.log`, `build.log` | Rust test, Mutter, D-Bus, and image-build diagnostics. |
| `app.log` | Completed core reads, artwork byte counts, search results, and preference readbacks. |
| `backend-instance.json`, `settings-path.txt` | Shared backend identity and isolated local settings path. |

To retain a separate run, pass another output directory under `build/`:

```sh
bash apps/linux/tests/e2e/run.sh apps/linux/build/e2e-repeat
```

Each run clears all five known PNG outputs before the build; failures never retain success screenshots in that output directory.
Retry and regression publish no canonical PNGs. Multi-scenario runs attempt every selected scenario, keep diagnostics in scenario-named subdirectories, and concatenate test output into `e2e.log`; `all` also copies the successful happy-path PNGs to the output root, removing them if any selected scenario fails.
Use a fresh directory to separate other diagnostic artifacts from prior runs.
The script prints errors and returns a nonzero exit status on failure.
It copies available container diagnostics before cleanup.

For harness failure checks, the script forwards `E2E_REQUIRED_LABEL`, `E2E_RESOLUTION`, and `E2E_APP` environment variables.
An absent extra label, invalid resolution, or absent application executable causes failure without success screenshots.
These variables affect only the external test harness, not application behavior.

Run the separate fault/recovery test with:

```sh
E2E_CATEGORY=retry bash apps/linux/tests/e2e/run.sh apps/linux/build/e2e-fault-recovery
```

This test gates the per-client forwarder until GTK shows a sanitized read error and Retry.
It then permits traffic to the same shared backend and invokes GTK's Retry action.
A per-client transport gate adds `Connection: close` to artwork GET requests, drops one stream and delays later responses. It preserves server response bytes and leaves RPC streams untouched. Metadata must remain usable during artwork delay; the deadline must stop further image requests. Refresh must recover actual server artwork. This journey requires enough published artwork to exercise multiple requests.
The test never kills or restarts the shared backend or injects successful catalog fixtures.
`fault-accessibility.xml` and `artwork-deadline-accessibility.xml` record the GTK trees after the two successful phases.
The scenario creates no PNGs. The Docker entry point bounds the container run.
Legacy `E2E_TEST=fault_recovery` remains an alias for `retry`; `E2E_TEST=home` maps to `all` to retain its former exhaustive coverage. Prefer `E2E_CATEGORY` for new commands. Do not combine conflicting selectors.

## Limits

These E2E tests cover one 1280×800 software-rendered Linux environment.
They do not exhaust keyboard paths, narrow-window layouts, assistive technologies, GPU drivers, or other clients.
Home expectations come from the configured server's current profile-linked progress; local UUIDs are selectors, not authentication.
The backend executes Semantic through exact vectors and reports lexical degradation explicitly. Every nonblank search uses a core RPC.
The screenshots support human inspection; the test does not compare pixels against a baseline.
The default app wrapper captures application output in `app.log`. WayDriver suppresses PipeWire/WirePlumber output.
Debian package repositories and base-image tags receive updates; the harness does not promise bit-identical images or screenshots.
