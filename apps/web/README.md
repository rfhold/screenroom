# Screenroom web demo

This Svelte SPA adapts the Moviee desktop layout: horizontal Home rows, an expandable in-flow sidebar, library catalogs, series/season browsing, title detail pages, and a separate search screen. The sidebar stays in the same shell on every screen. It pushes content sideways and stays visible during desktop scroll; on narrow screens it expands above the content without an overlay.
The mock catalog contains eight real films and two selected episodes from The Beverly Hillbillies. The Rust WASM client core reads metadata, artwork, credits, ratings, search and preferences from the gRPC-web mock server. Abstract posters and initials identify images that the catalog does not include; failed artwork loads show an unavailable state.
Cards open details or browse children; there are no accounts, media service connections, Play or Resume actions. Continue Watching progress, Up Next and addition order remain simulated mock state, not real watch history.
The static public website lives separately in [`../../website/`](../../website/).

## Run

Use Node `^22.17.0 || >=24.0.0` and pnpm `12.18.0` from the root package manifest.
Run these commands from the repository root:

```sh
pnpm install --frozen-lockfile
# In a separate terminal, from the repository root:
bash scripts/mock-server.sh
# Then:
pnpm --filter @screenroom/web dev
```

The default API endpoint is `http://127.0.0.1:50051`. Set `VITE_SCREENROOM_API_ENDPOINT` for a different server. Its `SCREENROOM_MOCK_PUBLIC_BASE` must match the endpoint and `SCREENROOM_MOCK_CORS_ORIGINS` must contain the exact web origin. `VITE_SCREENROOM_MOCK_PROFILE` selects a mock preference profile (default `web`); a browser session's `screenroom.mockProfile` sessionStorage value overrides it. These profiles are test state, not authentication.

`predev`, `prebuild` and `precheck` run [`scripts/prepare-core.mjs`](scripts/prepare-core.mjs), which always invokes the root WASM build script. Cargo checks source changes incrementally, not just output-file existence. Install Rust's `wasm32-unknown-unknown` target and the matching `wasm-bindgen` CLI from the [client API guide](../../docs/client-api.md#build-the-core). Generated JS/types/WASM stay under `target/client-core/wasm`. Vite bundles the imported WASM asset into production output. Generated binaries stay outside Git.

## E2E tests

```sh
pnpm --filter @screenroom/web test:e2e:install
pnpm --filter @screenroom/web test:e2e
```

Install Chromium once; later E2E runs reuse its binaries.
Playwright starts and stops a Vite development server on `127.0.0.1:4173` and a mock server on `127.0.0.1:50063`, waits for `/health`, and forwards teardown signals to the mock process. Server logs appear in the test output. Set `SCREENROOM_E2E_API_ENDPOINT` to use an already-running external mock instead; the suite does not start or stop that server. Each browser session gets a unique mock profile.
The port must be free; Playwright does not reuse another server.
Vite uses its normal dependency cache; the suite does not perform a production build.
Tests cover Home row order, details, sidebar geometry and keyboard access, breadcrumbs, URL restoration and browser history, library scope, hierarchy, search and filter guards, real refresh, absence of fake authentication, no full-page horizontal overflow, and no browser errors. Network checks allow only same-origin app assets and the exact mock origin's gRPC-web methods, artwork and health paths; same-origin artwork, external providers and playback remain forbidden. Additional tests assert WASM/gRPC-web/artwork responses, CatalogInfo and logo failure/retry, controlled server-response variants, stale search protection and profile-isolated preferences. Horizontal poster rows scroll intentionally.
Chromium runs desktop `1440×900` and mobile `390×844` viewport profiles with deterministic locale, dark color scheme, and reduced motion. Playwright uses one worker with `fullyParallel: false`; tests run one at a time across both profiles.
Both profiles share the same assertions and five viewport-only showcase captures, all scrolled to the top: Home with all libraries and the sidebar collapsed; Home with the sidebar expanded; Movies with the sidebar collapsed; All libraries search for “Bunny” with default Movies/Shows types, All catalogs and settings closed; and Big Buck Bunny opened from Movies with the sidebar collapsed and metadata information closed. The suite follows real navigation links and waits for fonts and visible images before capture. Regression tests retain image readiness checks for search settings, metadata information, people cards and credits without screenshots.
These profiles do not establish native mobile browser support or a complete accessibility audit.

Screenshots appear in `apps/web/e2e/build/screenshots/{home,sidebar,library,search,detail}-{desktop,mobile}.png`.
Only the `happy-path` tests write these ten canonical captures. They also assert actual WASM, gRPC-web, profile headers, artwork bytes, blob images, canonical data and geometry. Their request records appear in `apps/web/e2e/build/requests/happy-path-{desktop,mobile}.json`.
Playwright attachments and failure traces stay in `apps/web/e2e/build/results/`. Automatic screenshots stay off, including on failure. Failure traces can contain browser snapshots and embedded screenshots; the suite retains no successful traces.
The package wrapper first discovers tests with the actual CLI project, grep, path and category filters. Unknown categories and empty selections fail before cleanup, runtime preparation, server startup or test-artifact writes. After nonempty discovery, the suite removes all ten showcase captures so startup failures cannot leave stale export inputs. Successful happy-path tests replace captures; Playwright clears previous results.
The repository ignores these artifacts; production builds preserve them because they live outside `apps/web/build/`.
Remove `apps/web/e2e/build/` to clear all E2E artifacts.

Validate types and production output separately:

```sh
pnpm --filter @screenroom/web check
pnpm --filter @screenroom/web build
```

The E2E suite covers the development server, not the production output.
For a bounded production smoke test, build with the smoke endpoint and run the helper:

```sh
VITE_SCREENROOM_API_ENDPOINT=http://127.0.0.1:50064 pnpm --filter @screenroom/web build
node apps/web/scripts/test-production.mjs
```

The helper owns preview port `4176` and mock port `50064`, refuses an occupied endpoint, validates real WASM/CatalogInfo requests and decoded logo bytes, and stops both processes before returning. Its request record is `apps/web/e2e/build/requests/production.json`. It does not write screenshots.

### E2E categories

Set `E2E_CATEGORY` to select a category. Standalone runs default to `all`.

| Category | Covers |
| --- | --- |
| `happy-path` | Canonical navigation and ten viewport captures; two tests. |
| `retry` | Unavailable RPC and recovery through Retry; two tests. |
| `regression` | Pagination, history, preferences, server-response variants, rights, guards and other current assertions; 48 tests. |
| `all` | Every category; 52 tests. |

```sh
E2E_CATEGORY=happy-path pnpm --filter @screenroom/web test:e2e
E2E_CATEGORY=retry pnpm --filter @screenroom/web test:e2e
E2E_CATEGORY=regression pnpm --filter @screenroom/web test:e2e
E2E_CATEGORY=all pnpm --filter @screenroom/web test:e2e
```

Unknown and empty category values fail before server startup or WASM preparation. A filter with no tests also fails and preserves current artifacts. Each nonempty selection clears canonical captures first; retry and regression runs leave no canonical export inputs. Only tests with the `@happy-path` tag take screenshots, including within `all` runs. Root export orchestration selects `happy-path`; non-export orchestration defaults to `all`.

Each test declares its category through a Playwright tag. To add a category, extend `e2e/categories.mjs`, tag its tests and update this table. Keep canonical capture authority exclusive to `happy-path`.

After a happy-path run, `node --test apps/web/scripts/test-e2e-preflight.mjs` checks no-match artifact preservation with ten sentinel files. The check restores the original captures afterward.

## Structure

- [`src/lib/theme.css`](src/lib/theme.css) owns the isolated web theme.
- [`src/lib/components/Home.svelte`](src/lib/components/Home.svelte) owns the persistent shell and Home rows; neighboring components own the sidebar, breadcrumbs, search, details, and cards.
- [`src/lib/core/navigation.ts`](src/lib/core/navigation.ts) validates URL state against server libraries and item records and builds real navigation links; the shell reads SvelteKit's reactive `page.url`.
- [`src/lib/core/client.ts`](src/lib/core/client.ts) initializes WASM, maps server records for display, serializes preference updates and owns reference-counted artwork object URLs.
- [`src/lib/core/search.ts`](src/lib/core/search.ts) maps search controls to core RPC requests; it contains no search algorithm.
- [`src/lib/components/RemoteImage.svelte`](src/lib/components/RemoteImage.svelte) loads artwork bytes through the core and releases object URLs on cleanup. The image cache retains only mounted consumers.
- No catalog snapshots or artwork are bundled in the web app. Canonical server fixtures live under [`../../fixtures/catalog/`](../../fixtures/catalog/). All images, including provider branding, use core-loaded bytes and reference-counted blob URLs.
- [`e2e/home.spec.ts`](e2e/home.spec.ts) owns the interaction assertions and screenshot capture.
- [`vite.config.ts`](vite.config.ts) preserves the `index.html` SPA fallback.
- [`src/routes/+layout.ts`](src/routes/+layout.ts) disables server-side rendering.

## Mock-server behavior

Title details use a poster-and-summary header followed by full-width Cast, Crew and rights sections, then the credits disclosure. The detail surface is at most 1120 CSS pixels wide. At an available content width of 640 pixels or more the header uses a 200×300 poster beside the summary; below that it stacks a 160×240 poster above the summary. Provider badges use the returned identity, status and URL. Ratings retain their provider, scale and independent nullable vote count. No absent rating implies missing metadata or an invented provider connection. People wrap into square-photo cards with fully visible names and roles; character stills retain their labels.

Home always includes all libraries. Browse and search use the selected library. Breadcrumb links and browser history replace application Back buttons. Search retains the underlying browse hierarchy in its URL. Search waits 350ms after query edits and cancels stale timers. Movies and Shows default on; Episodes defaults off. At least one type and one search method remain selected.

The Catalog selector intersects catalog membership with the query, library, types and methods. Library IDs, labels and ordering come from ListLibraries. Available search methods, types, sources, maximum query length and semantic description come from GetSearchCapabilities. Protocol labels translate control names without adding unsupported options. Empty results are reported as empty results, not an inferred provider connection state. Fresh Search resets catalog scope and methods; reload, history and detail-return links preserve the current scope.

Search types persist through GetPreferences/UpdatePreferences in the selected mock profile, not browser localStorage or an account. URL types take precedence. Search methods reset on each fresh search. Search is debounced 350ms, cancels prior core requests and rejects stale generations. Server capabilities describe Semantic as a mock tag/keyword approximation, not embeddings. Title only disables Semantic and does not re-enable it when switched off.

Subtitle choices persist through masked UpdatePreferences calls without adding transcription or playback. Refresh invalidates core reads and rereads the current screen, CatalogInfo and search capabilities; success appears only after the reads complete. CatalogInfo supplies the ordered notices, global artwork manifest and branding, including images not visited in the UI. Item provenance notes, metadata retrieval time, title rights and metadata rights remain server-owned. There is no Welcome, Continue or logout flow. Initial RPC failures show an error and Retry, never a hidden local fixture fallback. Failed search requests likewise expose a retry action.

## URL navigation

All routes use the SPA's existing `/` path with readable query parameters:

- `/` — Home.
- `/?screen=browse&library=movies` — Movies in the current mock; library IDs are validated against the server response.
- `/?screen=browse&library=tv&series=beverly-hillbillies&season=beverly-hillbillies-s1` — the selected first-season episodes.
- `/?screen=search&library=tv&series=beverly-hillbillies&season=beverly-hillbillies-s1&q=Beverly&source=tmdb` — library-scoped search with catalog scope and breadcrumb hierarchy.
- `source=tmdb|jellyfin` — catalog scope; omission or an invalid value means All catalogs.
- `types=Movie,Series,Episode` — enabled search types; `methods=text,fuzzy,semantic` — enabled methods, with optional `titleOnly`.
- `title=big-buck-bunny` — title detail within the current route context; details render as pages, not modal overlays.

Links support normal browser navigation and modified-click/new-tab behavior. Reload and direct entry reconstruct the screen, hierarchy, query, filters, methods, and detail context. Item IDs and ancestry are validated asynchronously through core GetItem calls. Query edits replace the current history entry; links and filter changes create history entries. Sidebar expansion is UI-only state. Invalid screen/library/item relationships fall back to safe defaults; malformed filters revert to defaults and method guards prevent empty or incompatible selections. URL-backed filter selections take precedence over saved type preferences. Search settings contain every option: catalog, media types and methods. The inline disclosure starts closed beside the query. Escape closes it and returns focus to Settings; selected filters remain active.

## Metadata import

The browser reads the mock server through WASM only. Page loads never call TMDB, Jellyfin or external image services. External links require a user click. The root importer updates the canonical server fixture; it is not invoked by app startup or builds. No importer or fixture copies remain in this package.

To refresh the snapshot, provide `TMDB_READ_ACCESS_TOKEN` through your process environment. Never put the token in a file, shell command argument, source, fixture or browser variable. From the repository root:

```sh
pnpm --filter @screenroom/web import:tmdb
```

The native Node importer sends the token only as an HTTPS Authorization header to `api.themoviedb.org`. It verifies exact title/year matches and episode identities before the atomic fixture replacement. Missing credentials, authentication errors and incomplete fetches preserve the prior snapshot. Errors never include request headers, response bodies or credentials. No dependency or server accompanies the importer.

The snapshot includes retrieval time, TMDB identifiers, user vote averages on a ten-point scale, vote counts and native catalog links. No IMDb or Rotten Tomatoes scores appear. Zero votes means unrated; a missing import does not mean zero votes. Ratings do not represent age guidance or editorial recommendations. Metadata, ratings and API attribution follow [TMDB’s developer terms](https://developer.themoviedb.org/docs/faq), not film or artwork licenses. Commercial use needs a separate TMDB agreement.

Sprite Fright uses one explicit identity override: TMDB movie 891761. TMDB records 2024-04-06; the catalog retains Blender’s original 2021 release year. The snapshot preserves TMDB’s date and a discrepancy note. Acceptance requires the exact ID/title, director Matthew Luhn (1993493), and HTTPS homepage `studio.blender.org/films/sprite-fright/`. Other films still require exact title/year matches. The in-app credits explain this discrepancy.

Importer identity-guard tests use synthetic responses and no network, credentials or fixture writes:

```sh
pnpm --filter @screenroom/web test:importer
```

## Rights and attribution

The in-app **Data & artwork credits** disclosure renders the ordered notices and global artwork manifest from GetCatalogInfo, including the approved logo loaded through the core. It reads all paginated server titles on expansion for title and metadata rights, retrieval dates and provenance notes. Refresh rereads this content even when the disclosure stays open. Failed logo bytes show an unavailable state without a bundled fallback. The canonical fixture and artwork live under [`../../fixtures/catalog/`](../../fixtures/catalog/); CSS crops posters and people images for display and preserves the logo's aspect ratio.

- Original US public-domain films: Nosferatu, The General, His Girl Friday and Night of the Living Dead. Claims do not cover later restorations, new scores or every territory. Silent-film durations vary by edition.
- CC BY 3.0 films: Big Buck Bunny, Sintel and Tears of Steel, with Blender Foundation attribution. Logos and trademarks retain separate rights.
- Sprite Fright’s current official project page specifies CC BY 1.0, not CC BY 3.0.
- The Beverly Hillbillies includes only S01E01 and S01E02. Season 1 contains 36 episodes; this fixture is a selected subset. The catalog does not claim that the whole series or separate music is public domain.
- Four posters: one US public-domain Columbia Pictures poster and three CC BY 3.0 Blender Foundation posters.
- Four portraits: William Maanders, CC BY 3.0 Netherlands, from the official Big Buck Bunny team page.
- Two character stills: Duane Jones as Ben and Buster Keaton as Johnnie Gray, with US public-domain file-page provenance. These are not studio portraits.
- The official TMDB logo uses TMDB’s attribution permission, not a public-domain or Creative Commons license.

Public-domain status depends on jurisdiction and edition. Check local rights before reuse. The catalog includes horror and adult themes alongside family animation; it does not claim universal age suitability.
