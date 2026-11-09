# Index

| Path | Info |
| --- | --- |
| [README.md](README.md) | Repository purpose and scaffold status. |
| [docs/README.md](docs/README.md) | Documentation index. |
| [docs/components.md](docs/components.md) | Stated product scope and component boundaries. |
| [docs/client-api.md](docs/client-api.md) | Rust client core, mock API, artwork, and build contract. |
| [crates/](crates/) | Rust client core and mock server alongside service scaffolds. |
| [proto/](proto/) | Client API contracts and remaining service scaffolds; Buf configuration lives at the root. |
| [fixtures/catalog/](fixtures/catalog/) | Canonical mock catalog, artwork, and provenance snapshots. |
| [apps/apple/](apps/apple/) | SwiftUI shells and XcodeGen project definition. |
| [apps/android/](apps/android/) | Compose mobile and TV shells with the Gradle wrapper. |
| [apps/linux/](apps/linux/) | Independent Rust GTK4/libadwaita workspace. |
| [apps/web/](apps/web/) | Svelte SPA shell. |
| [website/](website/) | Static Cloudflare Pages website, not an SPA. |
| [compose.yaml](compose.yaml) | Local PostgreSQL configuration. |
| [.agents/skills/reviewing-changes/SKILL.md](.agents/skills/reviewing-changes/SKILL.md) | Repository-specific review risks, validation commands, and evidence requirements. |

# Hints

- Be mindful of disk usage from build outputs, caches, and test artifacts. When space is low, clean up obsolete generated outputs within the task's scope; preserve source files, user work, useful build caches, and diagnostics still needed for active work.
- Treat `docs/components.md` as scope authority, not proof of implemented features.
- Preserve scaffold boundaries unless the user explicitly requests implementation.
- Use installed `planning-changes` and `making-changes` for behavior changes.
- Supply `making-changes` with the local `reviewing-changes` skill's repository inputs.
- Use installed `agentic-documentation` for documentation and skill changes.
- Local work grants no commit, push, publication, deployment, or external mutation authority. Obtain explicit user approval for each action.
- When the user requests commits, use conventional commits by scope. Omit co-author trailers.
