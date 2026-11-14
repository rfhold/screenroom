# Index

| Path | Info |
| --- | --- |
| [Cargo.toml](Cargo.toml) | Independent GTK4/libadwaita workspace. |
| [src/backend.rs](src/backend.rs) | Shared-core application integration. |
| [src/settings.rs](src/settings.rs) | Private XDG settings persistence. |
| [src/profile_change.rs](src/profile_change.rs) | Profile-change isolation. |
| [tests/e2e.rs](tests/e2e.rs) | Feature-gated compositor E2E tests. |
| [README.md](README.md) | Linux prerequisites and E2E guidance. |

# Contracts

- Preserve this workspace and its lockfile independently of the root Rust workspace.
- Root workspace checks do not cover this application.
- Keep GTK objects on the main thread; transport tasks return owned domain data.
- Preserve cancellation and generation guards around shared-core requests.
- Keep XDG settings private and atomic; surface write failures.
- Do not add provider HTTP or private backend access to this shell.

# Hints

- Use `changing-client-contracts` for shared API effects.
- Linux checks require GTK4/libadwaita development libraries and a Linux environment.
- Select checks from [platform review inputs](../../.agents/skills/reviewing-changes/references/checks.md).
