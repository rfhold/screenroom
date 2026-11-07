# Index

| Path | Info |
| --- | --- |
| [README.md](README.md) | Repository purpose and scaffold status. |
| [docs/README.md](docs/README.md) | Documentation index. |
| [docs/components.md](docs/components.md) | Stated product scope and component boundaries. |
| [crates/](crates/) | Root Rust workspace: bare libraries, services, and worker. |
| [proto/](proto/) | Empty protobuf contracts; Buf configuration lives at the root. |
| [apps/apple/](apps/apple/) | SwiftUI shells and XcodeGen project definition. |
| [apps/android/](apps/android/) | Compose mobile and TV shells with the Gradle wrapper. |
| [apps/linux/](apps/linux/) | Independent Rust GTK4/libadwaita workspace. |
| [apps/web/](apps/web/) | Svelte SPA shell. |
| [website/](website/) | Static Cloudflare Pages website, not an SPA. |
| [compose.yaml](compose.yaml) | Local PostgreSQL configuration. |
| [.agents/skills/reviewing-changes/SKILL.md](.agents/skills/reviewing-changes/SKILL.md) | Repository-specific review risks, validation commands, and evidence requirements. |

# Hints

- Treat `docs/components.md` as scope authority, not proof of implemented features.
- Preserve scaffold boundaries unless the user explicitly requests implementation.
- Use installed `planning-changes` and `making-changes` for behavior changes.
- Supply `making-changes` with the local `reviewing-changes` skill's repository inputs.
- Use installed `agentic-documentation` for documentation and skill changes.
- Local work grants no commit, push, publication, deployment, or external mutation authority. Obtain explicit user approval for each action.
- When the user requests commits, use conventional commits by scope. Omit co-author trailers.
