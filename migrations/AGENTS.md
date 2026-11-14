# Index

| Path | Info |
| --- | --- |
| [../crates/screenroom-backend/src/db.rs](../crates/screenroom-backend/src/db.rs) | Embedded SQLx migration stream and startup lock. |
| [../docs/backend.md](../docs/backend.md) | Lifecycle, credentials, and migration operations. |
| [../crates/screenroom-backend/tests/](../crates/screenroom-backend/tests/) | Persistence, queue, admission, and migration invariants. |

# Contracts

- Treat applied migrations as immutable; append forward-only migrations.
- Preserve catalog, artwork, embeddings, users, links, instance identity, and receipt provenance across cutovers.
- Keep queue schema provider-neutral; domain tables own provider-specific admission and private connection references.
- Preserve short-lived migration pools, advisory locks, and restricted runtime pools.
- Keep startup migrations before listeners, worker claims, and readiness.
- Separate database credentials do not isolate process privilege.
- Keep runtime SQLx queries compatible with the forward schema; do not require compile-time database access.
- A migration file does not prove that a persistent instance applied it.

# Hints

- Database checks require explicit disposable targets; never substitute the persistent development instance.
- Follow [backend operations](../docs/backend.md) for lifecycle authority, not migration filenames or test commands.
