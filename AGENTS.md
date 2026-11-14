# Index

| Path | Info |
| --- | --- |
| [README.md](README.md) | Human overview and entry points. |
| [docs/README.md](docs/README.md) | Documentation index. |
| [docs/AGENTS.md](docs/AGENTS.md) | Documentation ownership and target-versus-current claims. |
| [docs/components.md](docs/components.md) | Stated product scope and component boundaries. |
| [docs/client-api.md](docs/client-api.md) | Public client core, local settings, scoped reads, artwork, and build contract. |
| [docs/backend.md](docs/backend.md) | Persistent backend, startup migrations, local lifecycle, search, artwork, and telemetry. |
| [docs/jellyfin.md](docs/jellyfin.md) | Managed connections, private API-key files, destination security, and setup limits. |
| [docs/testing.md](docs/testing.md) | Isolated checks, attached acceptance modes, E2E commands, and platform limits. |
| [crates/AGENTS.md](crates/AGENTS.md) | Backend, queue, providers, shared client, and generated Rust boundaries. |
| [proto/AGENTS.md](proto/AGENTS.md) | Wire contract ownership and generation. |
| [apps/AGENTS.md](apps/AGENTS.md) | Native clients and the Web SPA. |
| [website/AGENTS.md](website/AGENTS.md) | Static Cloudflare Pages website, not an SPA. |
| [scripts/AGENTS.md](scripts/AGENTS.md) | Generation, validation, and lifecycle entry points. |
| [migrations/AGENTS.md](migrations/AGENTS.md) | Forward-only persistent schema contracts. |
| [compose.yaml](compose.yaml) | Persistent local PostgreSQL, Garage, Envoy, catalog, user, and worker instance. |
| [.agents/skills/reviewing-changes/SKILL.md](.agents/skills/reviewing-changes/SKILL.md) | Repository-specific review risks, validation commands, and evidence requirements. |
| [.agents/skills/changing-indexing/SKILL.md](.agents/skills/changing-indexing/SKILL.md) | Queue, domain admission, worker, and publication change inputs. |
| [.agents/skills/changing-providers/SKILL.md](.agents/skills/changing-providers/SKILL.md) | Provider transport, private credentials, revisions, and profile change inputs. |
| [.agents/skills/changing-client-contracts/SKILL.md](.agents/skills/changing-client-contracts/SKILL.md) | Wire models, shared client, bindings, and consumer change inputs. |

# Hints

- Read the nearest scoped `AGENTS.md` before edits.
- Preserve source, prior dirty work, useful caches, and active diagnostics when disk space constrains builds.
- Treat `docs/components.md` as scope authority, not proof of implemented features.
- Runtime mocks, TMDB catalog fixtures, demo-user bootstrap, older maintenance APIs, and browser operation-journal replay are not supported product interfaces.
- Web Jellyfin setup uses shared operator methods, private backend key files, and private database references; native apps supply no onboarding UI.
- Verify implementation against source before claims of completion. Setup support does not prove live deployment or provider acceptance.
- Treat local user UUIDs and private RPC networks as selectors and topology, not authentication.
- Preserve forward-only applied migrations, persistent instance identity, operator state, and runtime keys.
- Obtain explicit approval before backend lifecycle changes, provider imports, or macOS GUI automation.
- Preserve unrelated scaffold boundaries.
- Use installed `planning-changes` and `making-changes` for behavior changes.
- Supply `making-changes` with the local `reviewing-changes` skill's repository inputs.
- Use installed `agentic-documentation` for documentation and skill changes.
- Local work grants no commit, push, publication, deployment, or external mutation authority. Obtain explicit user approval for each action.
- When the user requests commits, use conventional commits by scope. Omit co-author trailers.
