# Screenroom website

The static project website is separate from the Svelte web client in `apps/web/`.
SvelteKit prerenders every route with client JavaScript disabled.

## Architecture copy

The [component outline](../docs/components.md) owns intended backend responsibilities and the public deployment contract.
The website Docs page distinguishes the persistent local backend from production deployments and future provider integrations.
Catalog and user expose local gRPC and gRPC-Web reads over PostgreSQL. A durable worker indexes managed Jellyfin catalogs.
The [playback gateway](../docs/playback.md) manages Jellyfin playback sessions and serves media to clients.
The [backend guide](../docs/backend.md) owns local startup and shared telemetry configuration.
Their intended deployment shares one public DNS hostname, with ingress routes by gRPC service/method path or HTTP path.
The intended production contract requires independent service authentication; the local implementation supplies no authentication verifier.
Local UUID selectors and private RPC networks do not authenticate callers.

## Theme

`src/lib/theme.css` defines the dark theme’s semantic colors, spacing, typography, radii, and layout widths.
The root layout imports it once and owns the single main landmark.
`SiteHeader.svelte` and `SiteFooter.svelte` provide shared navigation and site chrome.

Use the shared `container`, `eyebrow`, `lead`, `actions`, `button`, `section`, `notice`, and `prose` classes for page structure.
Keep page-specific layout rules scoped, and use semantic tokens instead of new page palettes.
The `motif` utility provides the subtle line grid; keep it decorative and clear of dense text.

## Showcase

`src/lib/showcase.ts` defines client groups, screenshot flows, labels, and paths.
The showcase layout uses the shared theme and preserves screenshot aspect ratios.
Keep thumbnail images contained and retain links to the full original PNGs.

The 45 browse PNGs retain their original fixture-backed mock-server capture provenance.
They do not establish persistent-backend validation, current UI state, production providers, or playback support.
Current client E2E suites attach to one persistent backend with typed offline adapters; that cutover does not recapture these images.
The website serves static PNGs only; it needs no API client JavaScript or backend at runtime.

### Browse journey

From the repository root, refresh selected browse captures with:

```sh
bash scripts/e2e.sh --update-showcase
```

The root runner prepares core products sequentially, then starts the client families concurrently.
Showcase updates default to category `happy-path`. Use `--category all` to require every category before export.
`--category` overrides `E2E_CATEGORY`; without either, runs that do not update captures default to `all`.
The runner rejects unknown or empty categories and rejects `retry` or `regression` exports before tests start.
The root runner owns one named persistent Compose instance unless `SCREENROOM_BACKEND_ATTACH=1` explicitly selects caller-owned attachment.
All families use the same descriptor and identity. Linux forwards bytes to the same edge through the Compose client network.
Endpoint aliases must match the descriptor origin; ordinary teardown preserves volumes, keys, and the instance UUID.
The [E2E guide](../docs/testing.md#client-e2e-commands) defines endpoint variables, cache checks, prerequisites, and cancellation behavior.
Every selected suite must pass, and every selected PNG must validate, before the root runner copies captures into `static/showcase/`.
Apple exports require exactly one named success attachment per flow from the current run.
Apple diagnostics persist under `apps/apple/build/e2e/runs/`, even if capture export fails.
`E2E_APPLE_RUN_DIR` overrides that location and requires an absolute, new directory with a parent that exists.
The root runner preserves caller-supplied artifact paths; it removes only its temporary export stage.
The default selection exports 40 PNGs and preserves the five macOS captures.
`E2E_MACOS=1 bash scripts/e2e.sh --update-showcase` includes macOS and exports all 45 PNGs.
macOS tests require fresh user approval and `SCREENROOM_MACOS_GUI_READY=1`; explicit opt-in runs them before the concurrent batch.
For unattended batches, run macOS separately when ready for desktop automation.
Family arguments, such as `apple web`, limit both tests and capture updates.

### Playback journey

Playback uses the same shell pages and a separate Playback flow.
It never relabels the historical browse images as real-provider evidence.

Use an explicit title selector from the attached installation:

```sh
SCREENROOM_BACKEND_ATTACH=1 \
SCREENROOM_BACKEND_PROJECT=screenroom-jellyfin-local \
SCREENROOM_BACKEND_PORT=50052 \
SCREENROOM_API_ENDPOINT=http://127.0.0.1:50052 \
SCREENROOM_SHOWCASE_ITEM_ID='<selected item ID>' \
SCREENROOM_SHOWCASE_TITLE='Nosferatu' \
SCREENROOM_SHOWCASE_YEAR=1922 \
bash scripts/e2e.sh --update-showcase --journey playback --category happy-path
```

The installation descriptor supplies the instance identity and user selector.
Playback requires caller-owned backend attachment; it does not start a backend.
Use the descriptor's public endpoint; the command above selects the local port 50052 installation.
Family arguments limit the capture and export scope.
Playback runs families and profiles sequentially.
The playback journey accepts only `happy-path`.

macOS requires fresh GUI automation approval.
After approval, add `E2E_MACOS=1 SCREENROOM_MACOS_GUI_READY=1` to the environment.
An explicit macOS selection runs first.

Each producer writes a fresh `playback/manifest.json` beneath its normal E2E output.
The root exporter validates instance, title, platform, freshness, state metadata, and PNGs before website writes.
It updates `static/showcase/<shell>/playback/` and `src/lib/showcase-playback.json` together.
It preserves unselected platform metadata and historical browse assets.
Old standalone audit images never qualify as current-run export input.

An absent platform displays “No playback screenshots for this app.”
Exported entries supply title, year, runtime, viewport, outcome, state status, observations, and provenance.
Each page previews up to three playback screenshots. The full capture list links to original PNGs and records reasons for states without an image.
An error screenshot documents an error, not successful playback.
A successful capture suite does not establish playback acceptance.

Only sanitized website metadata enters the static build.
Private selectors, credentials, media URLs, and backend identity do not belong in that metadata.
The site needs no runtime API, external asset, or client JavaScript.
Local capture and build commands grant no publication or deployment authority.

## Local checks

From the repository root:

```sh
pnpm --filter @screenroom/website check
pnpm --filter @screenroom/website build
node --test website/tests/showcase*.mjs
```

Preserve `src/routes/+layout.ts` static route settings and `static/404.html`.
Product copy must distinguish historical fixture screenshots, current-run playback observations, and production acceptance.
