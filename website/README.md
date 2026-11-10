# Screenroom website

The static project website is separate from the Svelte web client in `apps/web/`.
SvelteKit prerenders every route with client JavaScript disabled.

## Architecture copy

The [component outline](../docs/components.md) owns intended backend responsibilities and the public deployment contract.
The website Docs page distinguishes the persistent local backend from production deployments and future provider integrations.
Catalog and user expose functional public Tonic gRPC/gRPC-Web reads over PostgreSQL; a durable worker uses typed offline provider adapters.
The playback gateway executable serves empty routes; its protobuf package defines no RPCs.
The [backend guide](../docs/backend.md) owns local startup and shared telemetry configuration.
Their intended deployment shares one public DNS hostname, with ingress routes by gRPC service/method path or HTTP path.
The intended production contract requires independent service authentication; the local implementation supplies no authentication verifier.
Local UUID selectors and private RPC networks are not authentication. Live provider connections and playback remain outside the implementation.

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

The current 45 PNGs retain their original fixture-backed mock-server capture provenance.
They do not establish persistent-backend validation, current UI state, production providers, or playback support.
Current client E2E suites attach to one persistent backend with typed offline adapters; that cutover does not recapture these images.
The website serves static PNGs only; it needs no API client JavaScript or backend at runtime.

From the repository root, refresh selected captures with:

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
The [root guide](../README.md#client-e2e-tests) defines endpoint variables, cache checks, prerequisites, and cancellation behavior.
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

## Local checks

From the repository root:

```sh
pnpm --filter @screenroom/website check
pnpm --filter @screenroom/website build
```

Preserve `src/routes/+layout.ts` static route settings and `static/404.html`.
Product copy must distinguish historical fixture screenshots, implemented local backend behavior, and future production services and playback.
