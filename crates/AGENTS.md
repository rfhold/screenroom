# Index

| Path | Info |
| --- | --- |
| [screenroom-backend/src/](screenroom-backend/src/) | Domain repositories, persistence, search, storage, and admission. |
| [screenroom-queue/src/lib.rs](screenroom-queue/src/lib.rs) | Provider-neutral PostgreSQL queue primitives. |
| [screenroom-index-worker/src/](screenroom-index-worker/src/) | Stateless work execution and fenced domain commits. |
| [screenroom-index-operator/src/](screenroom-index-operator/src/) | Private loopback HTTP operator capability. |
| [screenroom-catalog-adapter/AGENTS.md](screenroom-catalog-adapter/AGENTS.md) | Provider boundary. |
| [screenroom-client-core/AGENTS.md](screenroom-client-core/AGENTS.md) | Shared public client and separate operator client. |
| [screenroom-proto/AGENTS.md](screenroom-proto/AGENTS.md) | Generated Rust wire contracts. |
| [screenroom-observability/src/](screenroom-observability/src/) | Optional telemetry, profiling, and transport observations. |
| [../docs/backend.md](../docs/backend.md) | Backend lifecycle and persistence guidance. |

# Contracts

- The root workspace includes `crates/*`; Linux owns a separate workspace and lockfile.
- Keep provider business rules outside `screenroom-queue`.
- Domain repositories own eligibility, source cooldowns, budgets, catalog transactions, and publication.
- Workers require valid claims, immutable run inputs, and fences before durable effects.
- Pin an embedding model only when work needs embeddings.
- Keep provider credentials, connection locators, and database handles outside public client models.
- UUIDs, profile selectors, and private RPC topology do not authenticate callers.
- Preserve empty playback routes unless the task explicitly changes that boundary.

# Hints

- Use `changing-indexing` for queue, worker, admission, or publication changes.
- Use `changing-providers` for provider connections or profile changes.
- Use `changing-client-contracts` for public or operator contract changes.
- Select focused checks from [review inputs](../.agents/skills/reviewing-changes/SKILL.md).
