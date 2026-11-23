# Screenroom Web

This Svelte SPA adapts the Moviee desktop layout: horizontal Home rows, an expandable in-flow sidebar, library catalogs, series/season browsing, title detail pages, and a separate search screen. The sidebar stays in the catalog shell. It pushes content sideways and stays visible during desktop scroll; on narrow screens it expands above the content without an overlay.
The Rust WASM client core reads metadata, artwork, credits, ratings, and search through the persistent backend's public gRPC-Web API.
Managed Jellyfin connections supply real provider data through backend adapters; the app contains no catalog snapshots or image bundles.
Abstract posters and initials identify absent images; failed artwork loads show an unavailable state.
Cards open details or browse children; Movie and Episode details expose gateway-backed **Play**.
There are no accounts or client-owned provider connections.
Maintenance in the shared sidebar supports backend-managed Jellyfin setup and canonical index controls through an opt-in attachment.
Watched snapshots remain separate from Continue Watching progress, Up Next, and addition order; none implies watched completion.
The static public website lives separately in [`../../website/`](../../website/).
The [wire contract](../../docs/client-api.md) places catalog RPCs in `screenroom.catalog.v1` and UserService in `screenroom.user.v1`.
Immutable ClientConfig contains endpoint, local user UUID selector, and timeout; scoped reads preserve source/library context.

## Playback

The [playback guide](../../docs/playback.md) owns session, identity, media, uncertainty, and security rules.
The player uses ordinary shared WASM `Client` methods, not the operator capability or Maintenance proxy.
Video, HLS, and subtitles use validated gateway media URLs outside gRPC/WASM; the browser never contacts Jellyfin.
Playback requires an enabled local user, a current personal Jellyfin link, and backend-owned Quick Connect authorization.
The gateway never substitutes the indexing profile for an absent personal link or enables Quick Connect automatically.
The [Jellyfin guide](../../docs/jellyfin.md#personal-playback-authorization) owns that prerequisite.

**Play** requests the live personal resume position, not the catalog percentage.
It grants a one-use in-memory launch intent and opens `/play/[itemId]` with source/library scope and a validated internal return path.
**Back to details**, **Close player**, and Escape return to that detail URL; browser Back restores the prior view.
Direct entry, reload, and browser Forward show an explicit **Play** button without creating a session.
The black stage preserves the video aspect ratio, with compact charcoal title and playback-control overlays.
Loading and unavailable states show a concise central message and relevant actions.
An optional **Fullscreen** action uses the browser API and reports unsupported or failed requests.
Labelled custom controls provide play, pause, seek, mute, and volume without auto-hiding.
**Playback settings** contains track choices and expandable session information with explicit check/retry actions.
Audio and subtitle choices renegotiate transport; subtitles distinguish WebVTT, burn-in, unavailable tracks, and Off.
The initial conservative profile targets H.264/AAC MP4 and HLS through native video or pinned `hls.js`.
Preparation uses a 30-second deadline; control uses 10 seconds, separate from three-second metadata calls.
Autoplay refusal exposes an explicit Play action.

Ordinary rerenders, viewport changes, and fullscreen changes preserve the player and its session.
Close, navigation away, profile mutation, and unmount stop media and submit the final known position.
Read-only **Check session state** preserves position and pause state for an unchanged transport.
Ambiguous outcomes trigger state checks, not automatic personal-report replay.
Explicit create/transport retries retain the same UUID and exact body only in memory.
Reload restores no playback authority or journal.
Control capabilities and media URLs never enter localStorage or navigation state.
The video sets anonymous cross-origin mode before URLs; the document and media paths use no-referrer.
Native player UIs remain excluded; Safari and real-provider compatibility need separate acceptance evidence.

## Run

Native SvelteKit routes are `/`, `/libraries`, `/libraries/[libraryId]`, `/titles/[itemId]`, `/search`, `/play/[itemId]`, and `/maintenance`.
IDs are opaque and URL-encoded. Source/library queries preserve read scope; search queries preserve terms and filters.
Old root-query links canonicalize without mutations. Missing titles, libraries, and invalid ancestry show Not found.
Browser Back/Forward restore bounded in-memory catalog/search pages and view position after metadata is ready; profile changes clear those snapshots.
The shared layout loads global styles on every route. Static hosts must serve the generated `index.html` fallback for deep routes.

Use Node `^22.17.0 || >=24.0.0` and pnpm `12.18.0` from the root package manifest.
Run these commands from the repository root:

```sh
pnpm install --frozen-lockfile
# In a separate terminal, from the repository root:
bash scripts/backend.sh up
# Then:
pnpm --filter @screenroom/web dev
```

Backend startup requires explicit approval and the [backend prerequisites](../../docs/backend.md#persistent-local-instance).
The default API endpoint is `http://127.0.0.1:50051`. `VITE_SCREENROOM_API_ENDPOINT` selects another approved instance origin.
Public artwork URLs must share that origin; backend CORS must permit the exact browser origin.
`VITE_SCREENROOM_USER_ID` selects a local UUID, not authentication. Empty means anonymous catalog access.
Jellyfin setup requires the actual enabled local user's UUID; an absent or invalid selector disables setup submission.
The backend creates that selector once; use the installation descriptor, not a fixed demo identity.
Vite accepts `SCREENROOM_API_ENDPOINT` and `SCREENROOM_USER_ID` aliases when explicit `VITE_` values are absent.
Production builds compile these public settings; preview environment changes do not update them.
Validated localStorage settings use an origin/endpoint/user/namespace key; failed writes retain in-memory choices and show a warning.

`predev`, `prebuild` and `precheck` run [`scripts/prepare-core.mjs`](scripts/prepare-core.mjs), which checks freshness and rebuilds stale artifacts through the root WASM build script. Cargo checks source changes incrementally, not just output-file existence. Install Rust's `wasm32-unknown-unknown` target and the matching `wasm-bindgen` CLI from the [client API guide](../../docs/client-api.md#build-the-core). Generated JS/types/WASM stay under `target/client-core/wasm`. Vite bundles the imported WASM asset into production output. Generated binaries stay outside Git.

## Maintenance

The optional web Maintenance screen uses a separate typed Rust WASM `OperatorClient`, not the ordinary public RPC client.
It provides Refresh, Pause, Resume and Retry, with policy settings, run history and queue details under Advanced settings.
Direct `/maintenance` entry does not depend on public catalog initialization.
Native bindings expose the capability, but native apps supply no Maintenance UI.

Maintenance is always discoverable; disabled attachments show an unavailable state and expose no operator API.
Opt-in requires both server-only `SCREENROOM_WEB_OPERATOR_TARGET` and `SCREENROOM_WEB_OPERATOR_ORIGIN`, plus a separately approved web restart.
The [maintenance guide](../../docs/index-maintenance.md#explicit-local-attachment) owns exact values, development/preview commands, and production build/preview configuration parity.
Both Vite servers provide the guarded same-origin proxy; static deployment alone provides none.
Every script on the opt-in origin gains trusted-local administrator capability; the proxy provides no authentication.

Choose **Add server**, enter a backend-reachable Jellyfin URL and administrator API key, then choose **Connect**.
Select an enabled profile and 1–32 distinct supported libraries, with at most one collections library.
**Save** creates the connection and saved policy without jobs; **Save and refresh** also admits the initial run.
The [Jellyfin guide](../../docs/jellyfin.md#web-setup) owns the full procedure, destination security, private files, and first-add limits.
URL/key changes invalidate discovery and selections; success, Cancel setup, and unmount clear the browser's key.
The browser retains the key only in memory. Backend private key files provide no encryption at rest.

An ambiguous save offers explicit **Retry save** with the same in-memory UUID and exact body; no automatic mutation retry exists.
Index controls use the same explicit in-session request discipline. Reload reads server state without another mutation.
Page entry and reload never read or replay older valid or corrupt browser journals.
There are no recovery tabs, UUID exports, old-origin migrations, or automatic configure-to-start continuations.
Refresh requires a saved enabled policy; without one, use Add server or explicit Save settings in Advanced.
The standalone dashboard and older maintenance APIs are not supported interfaces.
The [operator API boundary](../../docs/client-api.md#operator-capability) owns typed errors, cancellation, exact integers, and request ambiguity.
These instructions authorize no backend restart, provider work, worker activation, or runtime change.

## E2E tests

```sh
pnpm --filter @screenroom/web test:e2e:install
pnpm --filter @screenroom/web test:e2e
```

Install Chromium once; later E2E runs reuse its binaries.
Playwright owns only its Vite development server on `127.0.0.1:4176` and attaches to the shared persistent backend.
`SCREENROOM_E2E_WEB_PORT` overrides that port; the backend must allow the resulting browser origin through CORS.
It verifies descriptor identity and real readiness; it never starts or stops an API server.
`SCREENROOM_E2E_API_ENDPOINT` and `SCREENROOM_API_ENDPOINT` must match the descriptor origin when both are set.
Each browser context uses an isolated local settings namespace, not a mock profile or backend identity.
The port must be free; Playwright does not reuse another server.
Vite uses its normal dependency cache; the suite does not perform a production build.
Attached tests use actual source IDs and capability-driven reads, not fixed titles, rows, library IDs, or provider logos.
The [suite source](e2e/home.spec.ts) defines current assertions; a browser build does not prove coverage of every public interaction.
Network assertions restrict public reads to app assets and the configured backend's public RPC/artwork paths.
Direct provider images and S3 remain forbidden; this attached browse suite does not establish playback acceptance.
Chromium uses desktop `1440×900` and mobile `390×844` profiles, deterministic locale, dark color scheme, and reduced motion.
Playwright uses one worker and serial tests across both profiles.
These profiles establish neither native mobile browser support nor a complete accessibility audit.
Showcase export requires current suite output; historical static images prove no current provider integration or playback.
Playwright attachments and failure traces stay in `apps/web/e2e/build/results/`. Automatic screenshots stay off, including on failure. Failure traces can contain browser snapshots and embedded screenshots; the suite retains no successful traces.
The wrapper first discovers tests with the actual CLI project, grep, path, and category filters.
Unknown categories and empty selections fail before cleanup, preparation, server startup, or artifact writes.
After nonempty discovery, the wrapper removes canonical showcase inputs so startup failures cannot leave stale export inputs.
A happy-path tag alone creates no captures; export requires explicit capture code and complete current output.
The real-catalog showcase journey explicitly captures Home, sidebar, library, search, and detail for both profiles after verifying loaded artwork and scoped reads.
Playwright clears previous results.
The repository ignores these artifacts; production builds preserve them because they live outside `apps/web/build/`.
Remove `apps/web/e2e/build/` to clear all E2E artifacts.

Validate types and production output separately:

```sh
pnpm --filter @screenroom/web check
pnpm --filter @screenroom/web build
```

The E2E suite covers the development server, not the production output.
For an approved attached production smoke test, set `SCREENROOM_API_ENDPOINT` and `SCREENROOM_USER_ID` to the descriptor's endpoint and enabled user.
Keep those variables set for both commands:

```sh
pnpm --filter @screenroom/web build
node apps/web/scripts/test-production.mjs
```

Never substitute a demo selector.
Both endpoint aliases must match the descriptor, and any explicit `VITE_` values must match the artifact's intended installation.
The helper requires workspace dependencies, Chromium, a current WASM artifact, and the backend helper's Python 3/grpcurl/Buf prerequisites.
It checks prepared-core freshness, owns preview port `4176`, refuses an occupied port, and attaches without backend lifecycle changes.
The production artifact must match the attached endpoint and user; incompatible artifacts require a rebuild, not runtime substitution.
It checks WASM, source-driven Home, honest empty state, and core-loaded artwork.
No logo, row count, fixture equality, or title is mandatory.
The helper writes a sanitized request record without screenshots, then stops only its preview process.
These checks do not establish live provider, TLS, credential-volume, device, or onboarding acceptance.

### Isolated browser contracts

The [indexing configuration](e2e/indexing.config.ts) selects onboarding, index controls, and public-WASM fault tests against controlled local responses.
The [maintenance configuration](e2e/maintenance.config.ts) selects attachment and operator-boundary tests.
These suites use real shared WASM exports but isolated HTTP/wire test support, not a product mock adapter or attached Jellyfin dataset.
Their source defines coverage; passing isolated checks does not establish attached real-source acceptance.
The [testing guide](../../docs/testing.md) separates these layers from live installation, provider, and platform acceptance.

For isolated playback checks, use:

```sh
pnpm --filter @screenroom/web test:playback
node --test apps/web/tests/watched.test.mjs
```

`test:playback` prepares fresh owned WASM and runs controller plus mounted browser tests.
It requires Chromium and `ffmpeg` with `libx264`/AAC, alongside workspace dependencies and the WASM toolchain.
The browser cases decode generated owned MP4/HLS and load anonymous WebVTT across distinct isolated SPA/API origins.
They mount actual details and test desktop/narrow player geometry, modal isolation, exit focus/scroll, and session retention across viewport changes.
Their HTTP double supplies no product catalog and proves no live gateway-plus-provider behavior.
The [canonical checks](../../docs/testing.md#playback-checks) own the complete prerequisites, database boundary, and separately authorized real-provider cases.

### E2E categories

Set `E2E_CATEGORY` to select a category. Standalone runs default to `all`.

| Category | Covers |
| --- | --- |
| `happy-path` | Source-driven Home and browse flows. |
| `retry` | Unavailable RPC and recovery through Retry. |
| `regression` | Current public contract assertions from the suite source. |
| `all` | Every category. |

```sh
E2E_CATEGORY=happy-path pnpm --filter @screenroom/web test:e2e
E2E_CATEGORY=retry pnpm --filter @screenroom/web test:e2e
E2E_CATEGORY=regression pnpm --filter @screenroom/web test:e2e
E2E_CATEGORY=all pnpm --filter @screenroom/web test:e2e
```

Unknown and empty category values fail before server startup or WASM preparation.
A filter with no tests fails and preserves current artifacts. Each nonempty selection clears canonical showcase inputs first.
Capture code requires the `@happy-path` tag; retry and regression selections supply no canonical export inputs.
Root export orchestration selects `happy-path`; non-export orchestration defaults to `all`.

Each test declares its category through a Playwright tag. To add a category, extend `e2e/categories.mjs`, tag its tests and update this table. Keep canonical capture authority exclusive to `happy-path`.

`node --test apps/web/scripts/test-e2e-preflight.mjs` checks no-match artifact preservation with synthetic sentinel files and restores prior files afterward.

## Structure

- [`src/lib/theme.css`](src/lib/theme.css) owns the isolated web theme.
- [`src/lib/components/Home.svelte`](src/lib/components/Home.svelte) owns the persistent shell and Home rows; neighboring components own the sidebar, breadcrumbs, search, details, and cards.
- [`src/lib/core/navigation.ts`](src/lib/core/navigation.ts) validates URL state against server libraries and item records and builds real navigation links; the shell reads SvelteKit's reactive `page.url`.
- [`src/lib/core/client.ts`](src/lib/core/client.ts) initializes WASM, maps server records, validates local settings, and owns reference-counted artwork object URLs.
- [`src/lib/core/search.ts`](src/lib/core/search.ts) maps search controls to core RPC requests; it contains no search algorithm.
- [`src/lib/components/RemoteImage.svelte`](src/lib/components/RemoteImage.svelte) loads artwork bytes through the core and releases object URLs on cleanup. The image cache retains only mounted consumers.
- No catalog snapshots or artwork are bundled in the Web app. All images use core-loaded bytes and reference-counted blob URLs.
- [`src/lib/components/MaintenanceConnections.svelte`](src/lib/components/MaintenanceConnections.svelte) owns private-key entry and managed setup through the shared operator client.
- [`src/lib/core/operator-state.ts`](src/lib/core/operator-state.ts) retains exact in-memory request bodies for explicit same-session Retry.
- [`src/lib/components/Player.svelte`](src/lib/components/Player.svelte) owns video/HLS, track controls, and anonymous gateway media.
- [`src/lib/components/PlaybackScreen.svelte`](src/lib/components/PlaybackScreen.svelte) owns the viewport modal, focus, scroll lock, and fullscreen action.
- [`src/lib/core/playback.ts`](src/lib/core/playback.ts) owns in-memory playback requests, ordered reports, reconciliation, and final-position capture.
- [`src/lib/core/playback-lifecycle.ts`](src/lib/core/playback-lifecycle.ts) revokes the active player before profile mutations.
- [`e2e/home.spec.ts`](e2e/home.spec.ts) owns the attached public interaction assertions.
- [`vite.config.ts`](vite.config.ts) preserves the `index.html` SPA fallback.
- [`src/routes/+layout.ts`](src/routes/+layout.ts) disables server-side rendering.

## Backend and local settings behavior

The shared poster shows a top-right checkmark only when the server's provider-neutral `Item.watched` value is `true`.
Its accessible label is `Watched`; explicit `false` and unknown show no checkmark.
Progress bars remain independent, even at 100 percent. Series and seasons gain no inferred completion.
The watched overlay adds no manual watched controls or provider-specific display branches.
Playback reports write personal provider state through the gateway and invalidate local reads after confirmed effects.
Watched state comes through the core from persisted backend snapshots, not browser provider requests.
Rendered screens receive no push updates; refresh or another read obtains current state.
See the [watched contract](../../docs/client-api.md#watched-state) for unknown values, source selection, and freshness.
Backend acquisition requires separate [configuration and activation approval](../../docs/backend.md#watched-snapshots).

Title details use a poster-and-summary header followed by full-width Cast and Crew sections. The detail surface is at most 1120 CSS pixels wide. At an available content width of 640 pixels or more the header uses a 200×300 poster beside the summary; below that it stacks a 160×240 poster above the summary. Provider badges use the returned identity, status and URL. Ratings retain their provider, scale and independent nullable vote count. No absent rating implies missing metadata or an invented provider connection. People wrap into square-photo cards with fully visible names and roles; character stills retain their labels.

Home always includes all libraries. Browse and search use the selected library. Breadcrumb links and browser history replace application Back buttons. Search retains the underlying browse hierarchy in its URL. Search waits 350ms after query edits and cancels stale timers. Movies and Shows default on; Episodes defaults off. At least one type and one search method remain selected.

The Catalog selector intersects catalog membership with the query, library, types and methods. Library IDs, labels and ordering come from ListLibraries. Available search methods, types, sources, maximum query length and semantic description come from GetSearchCapabilities. Protocol labels translate control names without adding unsupported options. Empty results are reported as empty results, not an inferred provider connection state. Fresh Search resets catalog scope and methods; reload, history and detail-return links preserve the current scope.

Search types persist in validated version-1 localStorage settings, not a server profile or account. URL types take precedence.
Search methods reset on each fresh search. Search waits 350ms after query edits, cancels prior requests, and rejects stale generations.
Semantic uses exact backend vectors; explicit degradation records explain lexical fallback. Title only disables Semantic without automatic restoration.

No global subtitle preference exists. Ordinary settings files ignore the obsolete field.
Subtitle controls belong to the active playback session.
The shared core normalizes saved `tmdb` to `all` and preserves unrelated valid settings.
Library IDs are opaque; literal `movies` and `tv` values imply no special behavior.
Refresh invalidates core reads and rereads the current screen and search capabilities.
Success appears only after reads complete. The client requests neither footer-only CatalogInfo nor the global ListArtwork inventory.
Item provenance, metadata dates, title rights, and metadata rights remain server-owned. No Welcome, Continue, or logout flow exists.
Initial RPC and search failures expose errors and Retry, never hidden local fixture fallbacks.

## URL navigation

All routes use the SPA's existing `/` path with readable query parameters:

- `/` — Home.
- `/?screen=maintenance` — explicit operator capability; unavailable without the opt-in configuration above.
- `/?screen=browse&library=<public-library-id>` — browse a server-supplied library.
- `series=<public-series-id>&season=<public-season-id>` — hierarchy within the current library.
- `/?screen=search&library=<public-library-id>&q=<query>` — library-scoped search.
- `source=<supported-provider>` — capability-admitted catalog scope; omission or an invalid value means All catalogs.
- `types=Movie,Series,Episode` — enabled search types; `methods=text,fuzzy,semantic` — enabled methods, with optional `titleOnly`.
- `title=<public-item-id>` — title detail within the current route context; details render as pages, not modal overlays.

Links support normal browser navigation and modified-click/new-tab behavior. Reload and direct entry reconstruct the screen, hierarchy, query, filters, methods, and detail context. Item IDs and ancestry are validated asynchronously through core GetItem calls. Query edits replace the current history entry; links and filter changes create history entries. Sidebar expansion is UI-only state. Invalid screen/library/item relationships fall back to safe defaults; malformed filters revert to defaults and method guards prevent empty or incompatible selections. URL-backed filter selections take precedence over saved type preferences. Search settings contain every option: catalog, media types and methods. The inline disclosure starts closed beside the query. Escape closes it and returns focus to Settings; selected filters remain active.

## Provider data

Public and operator control use shared WASM clients; media uses the gateway HTTP routes.
Page loads never call Jellyfin, S3, or external image services directly.
Managed setup sends the key through the guarded operator proxy to the private backend, not to a browser provider adapter.
External links require a user click. App startup and builds perform no provider import.
Runtime product mocks, TMDB importers, fixture feeds, and demo-user bootstrap are not supported interfaces.
Provider acquisition requires separate operational approval; the [Jellyfin guide](../../docs/jellyfin.md) owns supported setup and key ownership.

## Rights and attribution

The app has no global attribution or technical-credit footer, footer-only requests, state, styles, or retries.
Backend CatalogInfo, artwork credits, provenance, and rights APIs remain available; no provider-credit UI is required.
Item details retain per-item source identifiers, external links, ratings, provider status, and people labels.
Item details omit title, metadata, and artwork rights panels, metadata retrieval dates, and provenance notes. Backend and core records retain that data.
CSS crops posters and people images.
Backend records retain provider provenance; clients invent neither notices nor blanket public-domain claims.
Rights depend on the actual source, jurisdiction, and edition. Check local rights before reuse.
Historical website images remain static illustrations, not evidence of current provider access or universal age suitability.
