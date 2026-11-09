# Screenroom for Linux

A native GTK4/libadwaita catalog interface, independent of the root Rust workspace.
The Rust client core retrieves its mock catalog, search results, preferences, and artwork through the local mock server.
It has no media files, authentication, subtitle engine, real-provider connection, or playback support.

## Interface

- Home contains five horizontal poster rows with fixture progress indicators.
- The sidebar expands within the layout and stays available across Home, libraries, search, and details.
- Libraries contain responsive poster grids; series open seasons, and seasons open episodes.
- Title details use a 200×300 poster beside the summary, provider row and synopsis, followed by full-width Cast, Crew and Title rights sections. Below 640 logical content pixels, the header stacks with a 160×240 poster. There are no Play or Resume controls.
- People use adaptive grids with 140×140 portraits or honest placeholders, wrapping names and roles, and labels identifying character stills.
- The provider row uses server identities, statuses, URLs, rating provider and scale. Its native disclosure shows vote counts, snapshot dates, and provider references; unknown votes, zero votes, and absent ratings remain distinct.
- Clickable breadcrumbs return to libraries, series, seasons, or search results.
- Search preserves its scope, query, and options across detail navigation.
- Search waits 350 milliseconds after query edits and supports clear, blank, and no-results states.
- Search types start from server preferences. At least one supported type stays enabled.
- Server capabilities determine the source, content-type and search-method choices, query limit and Semantic description. Controls live inside Settings, closed initially.
- Inline settings offer text, edit-distance fuzzy titles, and a keyword/tag approximation called Semantic.
- Title only disables Semantic. Its later removal does not restore Semantic automatically.
- Subtitle choices and search content types persist through the mock Preferences service, within an explicit mock profile.
- Refresh aborts prior loads and searches, invalidates core reads, and reloads server data. Loading, completion, and errors reflect actual reads; there is no mock refresh counter or authentication flow.

The app uses native GTK widgets, server artwork, and Cairo placeholders without a web view.
Its app-scoped CSS matches the neutral dark web design; the app requests libadwaita's dark color scheme.
Navigation state resets when the process exits. The mock server retains profile preferences until its process exits.

`src/catalog.rs` maps core records into thin UI records. The app does not embed catalog JSON or title artwork.
All catalog artwork, including branding, comes from core-loaded server bytes. There are no local catalog, provenance, or artwork bundles; native application icons, CSS, system glyphs and generic placeholders remain local.
The core owns all RPC and artwork HTTP requests; no API key participates.
A two-thread Tokio runtime executes requests and raster image decode outside GTK's main thread.
GTK's main context installs completed snapshots and creates native textures from ready pixels.
Request generations reject stale results; query edits abort superseded requests. View destruction cancels core requests.
Initial load, error, retry, empty results, and missing-image states never substitute embedded catalog data.

The Data & artwork credits disclosure renders the ordered CatalogInfo notices, complete global artwork inventory, logos identified by artwork kind, authors, licenses, source links and retrieval timestamp. Item title rights, independent metadata rights and timestamps, and provenance notes also come from server records, without title-specific rules.
Public-domain claims remain US- and edition-specific. Artwork and metadata have separate terms; character stills carry explicit labels.
The server supplies stable artwork references for credits, regardless of name variations.

## Native run

Use an edition-2024 Rust toolchain, `pkg-config`, GTK 4.10+ development libraries, and libadwaita development libraries.
The Docker build pins Rust 1.98.1 and Debian 13 (Trixie).
On Debian 13, the development packages are `libgtk-4-dev`, `libadwaita-1-dev`, and `pkg-config`.

From the repository root:

```sh
cargo run --locked -p screenroom-mock-server
```

In another terminal:

```sh
cargo run --manifest-path apps/linux/Cargo.toml --locked
cargo fmt --manifest-path apps/linux/Cargo.toml --all -- --check
cargo check --manifest-path apps/linux/Cargo.toml --locked
cargo test --manifest-path apps/linux/Cargo.toml --locked --bin screenroom-linux
```

The native run needs a desktop session and a reachable mock server.
`SCREENROOM_ENDPOINT` defaults to `http://127.0.0.1:50051`; `SCREENROOM_MOCK_PROFILE` defaults to `linux-demo`.
The core uses a three-second request timeout. Profiles isolate demo preferences, not accounts or authentication.
`src/main.rs` owns application setup; `src/home.rs` owns navigation, the sidebar, grids, and details.
`src/backend.rs` loads complete paginated core snapshots and decodes artwork.
Library IDs, titles, order, Home rows and membership come from server responses, not local media-kind filters. Navigation retains stable library and item IDs across reordered records.
Library, child, credit, and search reads consume every continuation before the UI reports complete results.
Page failures show an error instead of partial success; repeated tokens fail without an endless loop.
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
Its build context uses the repository root so path dependencies and mock fixtures enter the builder.
The Dockerfile-specific allowlist excludes unrelated applications, local builds, and repository metadata.
The runtime image contains only the app, test binary, mock server, helper scripts, and system runtime packages.
The mock server embeds its fixtures and starts inside the test container on loopback port 50051.
The runner checks `/health` before app startup and stops its server process on exit. No host server participates.
It compiles the integration test with `cargo test --locked --features e2e --test e2e --no-run`.
BuildKit retains Cargo registry, Git, and target caches between builds.
Source-only edits reuse compiled dependencies; test-only edits do not rebuild the app.
The runtime executes Cargo's compiled integration test binary with standard Rust test discovery and output, without Cargo or a Rust toolchain.
The `e2e` feature gates this test target; ordinary checks do not start a display.
WayDriver and its Mutter backend crates are development dependencies pinned to 0.3.10.
The image uses a non-root Mutter compositor, a headless output, Mesa software rendering, and GTK's Cairo renderer.
The app requires the Wayland backend; Mutter disables X11.
No X11 server, Xvfb, GPU device, privileged mode, host mount, or host display socket participates.
The container retains `--network none`; only its private loopback connects the app and mock server.
It drops all capabilities and limits memory and process count.

`E2E_CATEGORY` selects `happy-path`, `retry`, `regression`, or `all` (the standalone default).
Unknown categories and conflicting legacy selectors fail before output cleanup or Docker access.
Every scenario uses WayDriver `Session` and XPath locators over AT-SPI on a private D-Bus session.
`happy-path` runs one short native journey through the five canonical capture states, requiring real catalog, search, and artwork readiness.
`retry` runs the real unavailable-server recovery and artwork-404 scenario described below.
`regression` runs two independent scenarios: catalog/preferences and search/history.
Catalog coverage retains Home scroll response, all subtitle writes and profile isolation, library scope, series/season/episode navigation, breadcrumbs, title details, numeric ratings with unknown vote counts, character-still labeling, provider disclosure, and absent playback controls.
Search coverage retains keyboard query input, source and type filters, title-only constraints, default-option guards, Escape, clear/no-results states, global credits and real Refresh reads. It also rejects fake authentication controls and refresh counters.
Pending-search checks settle a query prefix, then observe the final edit concurrently with its keystroke and invoke the Search breadcrumb or Refresh.
Both actions must preserve the pending indicator and complete the original query.
Each locator has a five-second deadline; each scenario's assertion phase has a 55-second timeout.
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
| `sidebar.png` | Home with the expanded sidebar and subtitle controls. |
| `library.png` | Movies library with its eight-title responsive grid and collapsed sidebar. |
| `search.png` | All libraries search for `Bunny`, with default options, Settings closed and sidebar collapsed. |
| `detail.png` | Big Buck Bunny opened from Movies, at the top with provider information closed and sidebar collapsed. |
| `accessibility.xml` | Last available application accessibility tree, on success or assertion failure. |
| `e2e.log`, `build.log` | Rust test, Mutter, D-Bus, and image-build diagnostics. |
| `app.log` | Completed core reads, artwork byte counts, search results, and preference readbacks. |
| `mock-server.log`, `mock-health.txt` | Container-local mock lifecycle and health evidence. |

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

This test reserves the endpoint without an HTTP service until GTK shows the core deadline error and Retry.
It then starts the real mock router and invokes GTK's Retry action.
Test middleware returns HTTP 404 for one poster; assertions require its accessible placeholder, real titles, and another visible poster.
`fault-requests.log` records actual RPC and artwork HTTP responses. `fault-accessibility.xml` records the final GTK tree.
The scenario creates no PNGs and retains its own 55-second assertion deadline within the unchanged 75-second container limit.
Legacy `E2E_TEST=fault_recovery` remains an alias for `retry`; `E2E_TEST=home` maps to `all` to retain its former exhaustive coverage. Prefer `E2E_CATEGORY` for new commands. Do not combine conflicting selectors.

## Limits

These E2E tests cover one 1280×800 software-rendered Linux environment.
They do not exhaust keyboard paths, narrow-window layouts, assistive technologies, GPU drivers, or other clients.
The fixture progress values do not represent actual watch history.
The mock Search service executes Semantic as deterministic keyword/tag rules, not a model. Every nonblank search uses a core RPC.
The screenshots support human inspection; the test does not compare pixels against a baseline.
The default app wrapper captures application output in `app.log`. WayDriver suppresses PipeWire/WirePlumber output.
Debian package repositories and base-image tags receive updates; the harness does not promise bit-identical images or screenshots.
