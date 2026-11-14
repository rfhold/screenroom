# Index

| Path | Info |
| --- | --- |
| [apple/AGENTS.md](apple/AGENTS.md) | SwiftUI and UniFFI integration. |
| [android/AGENTS.md](android/AGENTS.md) | Compose and Kotlin/JNI integration. |
| [linux/AGENTS.md](linux/AGENTS.md) | GTK4/libadwaita and independent Rust workspace. |
| [web/AGENTS.md](web/AGENTS.md) | Svelte SPA, WASM, and same-origin operator proxy. |
| [../crates/screenroom-client-core/AGENTS.md](../crates/screenroom-client-core/AGENTS.md) | Shared client owner. |

# Contracts

- Keep server-data behavior in the shared client; platform UI owns presentation and native interaction.
- Do not call Jellyfin or database services directly from application shells.
- Preserve explicit loading, empty, error, cancellation, and retry states.
- Reject stale responses after scope, profile, or client-generation changes.
- Keep source metadata separate from personal watched state.
- The approved setup target starts with Web Jellyfin API-key entry; this does not imply completed native setup support.
- Operational agent approvals do not define application confirmation screens.

# Hints

- Use `changing-client-contracts` when shared models or API behavior affects a shell.
- Keep the [static website](../website/AGENTS.md) distinct from the Web SPA.
