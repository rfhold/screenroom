# Screenroom

Domain: **screenroom.tv**.

Screenroom pairs native client interfaces and a web SPA with a shared Rust client core and a development mock API.

The core and mock server implement catalog reads, search, artwork transport, and anonymous mock preferences.
They do not implement authentication, provider connections, library sync, or playback. The website remains separate from the web client.
The client contract requires all catalog content, media artwork, provider branding, and attribution notices through the mock API and Rust core.
Client shells own presentation, not bundled catalog fixtures or simulated authentication and refresh outcomes.

See the [client API guide](docs/client-api.md) for architecture and local build/run procedures.
The [component outline](docs/components.md) separates implemented components from intended scope.
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

Apple runs iOS, iPadOS, and tvOS by default; macOS XCUITest is opt-in because it uses the desktop.
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

The root runner owns one unique-port loopback mock for Apple, Android, and web.
Set `SCREENROOM_API_ENDPOINT` to opt into a caller-owned server; the runner never stops that server.
`SCREENROOM_E2E_API_ENDPOINT` also selects the external server; both variables must match if set together.
External servers need matching artwork URLs and CORS for `http://127.0.0.1:4173`.
Android endpoints require an explicit port for ADB reverse. Linux retains its separate network-isolated container mock.
Android uses a unique run prefix for UI and contract profiles, including with external servers.
`SCREENROOM_CORE_PREPARED=1` is an internal verification-only mode, not a build bypass.

When `E2E_MACOS=1`, macOS runs exclusively before the concurrent families, while the user is ready for desktop automation.
For an unattended batch, leave macOS disabled and run its standalone command separately when ready.

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
Native requirements include Xcode/XcodeGen, Android JDK 17 and SDK 37 with an AVD, Docker, GNU timeout, and jq.
Showcase export also requires the standard `file` utility.
Root orchestration requires Node.js on `PATH`, including native-only selections, for port allocation and owned-process cleanup.
Use the Node version range in root `package.json`.
Each guide lists its exact environment, independent suite command, and screenshot paths:

- [Apple: iOS, iPadOS, macOS, and tvOS](apps/apple/README.md)
- [Android and Android TV](apps/android/README.md)
- [Linux: headless Wayland in Docker](apps/linux/README.md)
- [Web SPA](apps/web/README.md)
