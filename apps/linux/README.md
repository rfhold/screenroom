# Screenroom for Linux

A native GTK4/libadwaita catalog interface, independent of the root Rust workspace.
The Rust client core retrieves catalog data, search results, and artwork through the persistent local backend with offline typed adapters.
It has no media files, authentication, subtitle engine, real-provider connection, or playback support.
The [wire contract](../../docs/client-api.md) places catalog RPCs in `screenroom.catalog.v1` and UserService in `screenroom.user.v1`.
Immutable ClientConfig holds endpoint, local user UUID selector, and timeout. Scoped requests preserve source/library context through details and continuation pages.

## Interface

- Home contains server-defined poster rows; mapped local users receive typed mock Continue Watching state.
- The sidebar expands within the layout and stays available across Home, libraries, search, and details.
- Libraries contain responsive poster grids; series open seasons, and seasons open episodes.
- Title details use a 200×300 poster beside the summary, provider row and synopsis, followed by full-width Cast, Crew and Title rights sections. Below 640 logical content pixels, the header stacks with a 160×240 poster. There are no Play or Resume controls.
- People use adaptive grids with 140×140 portraits or honest placeholders, wrapping names and roles, and labels identifying character stills.
- The provider row uses server identities, statuses, URLs, rating provider and scale. Its native disclosure shows vote counts, snapshot dates, and provider references; unknown votes, zero votes, and absent ratings remain distinct.
- Clickable breadcrumbs return to libraries, series, seasons, or search results.
- Search preserves its scope, query, and options across detail navigation.
- Search waits 350 milliseconds after query edits and supports clear, blank, and no-results states.
- Search types start from local settings. At least one supported type stays enabled.
- Server capabilities determine the source, content-type and search-method choices, query limit and Semantic description. Controls live inside Settings, closed initially.
- Inline settings offer PostgreSQL full-text search, title similarity, and exact vector Semantic search with explicit lexical degradation.
- Title only disables Semantic. Its later removal does not restore Semantic automatically.
- Subtitle choices and search content types persist in validated version-1 private XDG JSON through atomic file replacement.
- Refresh aborts prior loads and searches, invalidates core reads, and reloads server data. Loading, completion, and errors reflect actual reads; there is no mock refresh counter or authentication flow.

The app uses native GTK widgets, server artwork, and Cairo placeholders without a web view.
Its app-scoped CSS matches the neutral dark web design; the app requests libadwaita's dark color scheme.
Navigation state resets when the process exits. Device-local preferences survive app and backend restarts independently.
The default settings path is `$XDG_CONFIG_HOME/screenroom/settings.json`, or `$HOME/.config/screenroom/settings.json`.
`SCREENROOM_SETTINGS_PATH` overrides the path; `SCREENROOM_E2E_SETTINGS_NAMESPACE` isolates test files.
The store reports write failures and preserves the current in-memory choices.

`src/catalog.rs` maps core records into thin UI records. The app does not embed catalog JSON or title artwork.
All catalog artwork, including branding, comes from core-loaded server bytes. There are no local catalog, provenance, or artwork bundles; native application icons, CSS, system glyphs and generic placeholders remain local.
The core owns all RPC and artwork HTTP requests; no API key participates.
A two-thread Tokio runtime executes requests and raster image decode outside GTK's main thread.
GTK's main context installs completed snapshots and creates native textures from ready pixels.
Request generations reject stale results; query edits abort superseded requests. View destruction cancels core requests.
Initial load, error, retry, empty results, and missing-image states never substitute embedded catalog data.

The Data & artwork credits disclosure renders bounded CatalogInfo notices and branding, plus the complete paginated ListArtwork inventory on demand.
Authors, licenses, source links, title rights, metadata rights, and provenance remain server-owned, without title-specific rules.
The global timestamp reflects eligible persisted provider snapshots, not the request clock or visited-title subset.
Public-domain claims remain US- and edition-specific. Artwork and metadata have separate terms; character stills carry explicit labels.
The server supplies stable artwork references for credits, regardless of name variations.

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
The core uses a three-second request timeout.
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
`retry` runs the real unavailable-server recovery and artwork-404 scenario described below.
`regression` runs two independent scenarios: catalog/preferences and search/history.
Catalog coverage includes Home scroll, local subtitle persistence and namespace isolation, scoped hierarchy, breadcrumbs, metadata, role credits, provider disclosure, and absent playback controls.
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

This test gates the per-client forwarder until GTK shows a real transport error and Retry.
It then permits traffic to the same shared backend and invokes GTK's Retry action.
A per-client fault path returns artwork 404; assertions require an honest placeholder, real titles, and another visible poster.
The test never kills or restarts the shared backend or injects successful catalog fixtures.
`fault-requests.log` records actual RPC and artwork HTTP responses. `fault-accessibility.xml` records the final GTK tree.
The scenario creates no PNGs and retains its own 55-second assertion deadline within the unchanged 75-second container limit.
Legacy `E2E_TEST=fault_recovery` remains an alias for `retry`; `E2E_TEST=home` maps to `all` to retain its former exhaustive coverage. Prefer `E2E_CATEGORY` for new commands. Do not combine conflicting selectors.

## Limits

These E2E tests cover one 1280×800 software-rendered Linux environment.
They do not exhaust keyboard paths, narrow-window layouts, assistive technologies, GPU drivers, or other clients.
The fixture progress values do not represent actual watch history.
The backend executes Semantic through exact vectors and reports lexical degradation explicitly. Every nonblank search uses a core RPC.
The screenshots support human inspection; the test does not compare pixels against a baseline.
The default app wrapper captures application output in `app.log`. WayDriver suppresses PipeWire/WirePlumber output.
Debian package repositories and base-image tags receive updates; the harness does not promise bit-identical images or screenshots.
