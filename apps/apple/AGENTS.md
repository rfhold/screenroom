# Index

| Path | Info |
| --- | --- |
| [project.yml](project.yml) | XcodeGen source for iOS, tvOS, and macOS targets. |
| [Sources/Home/](Sources/Home/) | SwiftUI catalog state and views. |
| [scripts/prepare-core.sh](scripts/prepare-core.sh) | Shared core artifact preparation. |
| [Tests/](Tests/) | Local state and cancellation checks. |
| [README.md](README.md) | Platform prerequisites and E2E entry points. |

# Contracts

- Change `project.yml`, not the generated Xcode project.
- Keep Swift bindings and the XCFramework coupled to the same core source.
- Preserve UniFFI cancellation patches, buffer ownership, and future lifecycle guarantees.
- Keep UI state on its actor; reject stale task results after client or profile changes.
- Preserve native focus behavior across touch, keyboard, and tvOS remote input.
- Keep local settings writes atomic and explicit on failure.

# Hints

- Use `changing-client-contracts` for binding changes.
- Build authority grants no macOS GUI automation authority; require explicit approval and `SCREENROOM_MACOS_GUI_READY=1`.
- Use [platform review checks](../../.agents/skills/reviewing-changes/references/checks.md), not a desktop automation fallback.
