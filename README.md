# Screenroom

Domain: **screenroom.tv**.

Screenroom pairs native client interfaces and a web SPA with a shared Rust client core and a persistent local backend.

PostgreSQL stores the shared catalog, local users, durable ingest jobs, search documents, and embeddings. Garage stores private artwork bytes.
Typed mock adapters supply offline Jellyfin and TMDB inputs; live provider connections, authentication, and playback remain outside the implementation.
Clients store preferences locally. The website remains separate from the web client.
The client contract requires all catalog content, media artwork, provider branding, and attribution notices through the backend and Rust core.
Client shells own presentation, not bundled catalog fixtures or simulated authentication and refresh outcomes.

See the [client API guide](docs/client-api.md) for architecture and local build/run procedures.
The [component outline](docs/components.md) separates implemented components from intended scope.
Catalog and user services expose functional public reads; the worker persists typed adapter data and artwork.
The playback gateway still has no business RPC routes.
The [backend development guide](docs/backend.md) covers persistent local startup, migrations, search, artwork, and shared telemetry.
The [public deployment contract](docs/components.md#intended-public-deployment) defines their shared hostname, ingress routing, and independent authentication requirements.
The [documentation index](docs/README.md) routes to architecture and client guides.

## Client E2E tests

Run every client family from the repository root:

```sh
bash scripts/e2e.sh
```

Select one or more families with `bash scripts/e2e.sh apple web`.
Use `--category happy-path`, `--category retry`, or `--category regression` to select a test group;
`--category all` runs every group. The root and standalone family runners default to `all`,
except the root defaults to `happy-path` with `--update-showcase`.
`E2E_CATEGORY` sets the category when no CLI category is supplied: CLI overrides environment, then defaults apply.
Unknown or empty categories fail before preparation. For example:

```sh
bash scripts/e2e.sh --category retry apple web
bash scripts/e2e.sh --category happy-path
```

Apple runs iOS, iPadOS, and tvOS by default; macOS XCUITest requires fresh desktop approval and `SCREENROOM_MACOS_GUI_READY=1`.
After approval, prefix each macOS command below with that readiness variable.
Use `E2E_MACOS=1 bash scripts/e2e.sh` (or add `apple`) to include all four Apple profiles,
or `bash apps/apple/scripts/e2e.sh macos` to run only macOS.
`E2E_MACOS` accepts only `0` (the default when unset) or `1`; other values fail before any suite runs.
The runner prepares fresh Apple, Android, and WASM core products sequentially before any suite starts.
Content fingerprints permit cache reuse; suite-time checks reject stale products instead of concurrent regeneration.
Apple simulators, Android, Linux, and web then run concurrently, with one log per family.
Apple profiles and Android profiles stay sequential within their families; web uses one Playwright worker.
Absent tools cause errors, not skipped tests. Failed families prevent showcase export after the other bounded suites finish.
The runner retains logs under `build/e2e/runs/` and stops its owned process trees on exit or cancellation.
Each family reports completion independently, with its category, exit status, elapsed seconds, and log path.
Per-group `.summary` files retain the same category, status, and measured duration.
GNU timeout bounds each family to 40 minutes, with a 20-second kill grace.

The root runner owns one persistent named Compose instance unless `SCREENROOM_BACKEND_ATTACH=1` selects a caller-owned instance.
Standalone family runners require that instance; none starts a separate API server.
Start the backend separately only with approval: `bash scripts/backend.sh up`.
Use `SCREENROOM_BACKEND_ATTACH=1 bash scripts/e2e.sh` to attach without backend teardown.
The [backend guide](docs/backend.md#persistent-local-instance) defines lifecycle commands and prerequisites.
`SCREENROOM_API_ENDPOINT` and `SCREENROOM_E2E_API_ENDPOINT` must match the instance origin and each other.
The default origin is `http://127.0.0.1:50051`; `SCREENROOM_BACKEND_PORT` selects an explicit alternate port.
Every family uses the same secret-free descriptor and instance identity.
`SCREENROOM_USER_ID` selects a local UUID, not authentication; the root defaults to the bootstrap Alice UUID.
Android uses same-port ADB reverse. Linux uses the Compose client network and a byte-only loopback forwarder.
Unique local settings namespaces isolate test preferences, not backend identities.
`SCREENROOM_CORE_PREPARED=1` is an internal verification-only mode, not a build bypass.
The root owner lock rejects concurrent root runs. Cleanup stops only resources that the invocation owns.
Ordinary backend teardown preserves named volumes, keys, and the instance UUID.

When `E2E_MACOS=1`, macOS runs exclusively before the concurrent families, while the user is ready for desktop automation.
For an unattended batch, leave macOS disabled. Obtain fresh approval before any macOS GUI automation.

To refresh website showcase screenshots after the selected E2E suites pass:

```sh
bash scripts/e2e.sh --update-showcase
bash scripts/e2e.sh --update-showcase apple web
```

Without the flag, tests never update the website. With it, all selected suites must pass and every selected PNG
must validate before any file under `website/static/showcase/` changes. Unselected families keep their images.
Exports require `happy-path` or `all`; `retry` and `regression` are rejected before any tests start.
Export errors or cancellation restore any selected images that the copy phase already changed.
If restoration fails, the runner reports the backup directory and preserves it for recovery.
Each selected profile exports Home, expanded sidebar, Movies library, Search, and title detail captures.
Native images use `{platform}/{home,sidebar,library,search,detail}.png`. Web Home keeps `web/desktop.png`
and `web/mobile.png`; other web flows use `web/{desktop,mobile}-{sidebar,library,search,detail}.png`.
Apple exports use only the current run's named `{flow}-success` attachments. The export rejects absent or ambiguous captures.
Default Apple showcase updates preserve all five current macOS images. Use
`E2E_MACOS=1 bash scripts/e2e.sh --update-showcase apple` to require all five flows from all four current-run Apple profiles.
The other suites clear their previous success captures before each run.
The app guides define each suite's integration coverage; screenshots do not establish provider integration or playback support.

Install each family's prerequisites first; the runner does not install SDKs, packages, or browsers.
Web requires the root Node version, pnpm `12.18.0`, workspace dependencies, and Playwright Chromium.
If the global pnpm launcher fails to select that version, use `npx --yes pnpm@12.18.0` instead.
Native requirements include Xcode/XcodeGen, Android JDK 17 and SDK 37 with an AVD, Docker, GNU timeout, and jq.
Showcase export also requires the standard `file` utility.
Root orchestration requires Node.js on `PATH`, including native-only selections, for descriptor reads and owned-process cleanup.
Use the Node version range in root `package.json`.
Each guide lists its exact environment, independent suite command, and screenshot paths:

- [Apple: iOS, iPadOS, macOS, and tvOS](apps/apple/README.md)
- [Android and Android TV](apps/android/README.md)
- [Linux: headless Wayland in Docker](apps/linux/README.md)
- [Web SPA](apps/web/README.md)
