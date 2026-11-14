# Index

| Path | Info |
| --- | --- |
| [src/jellyfin.rs](src/jellyfin.rs) | Bounded Jellyfin transport. |
| [../screenroom-backend/src/connections.rs](../screenroom-backend/src/connections.rs) | Managed runtime connection owner and authenticated server-identity checks. |
| [../screenroom-backend/src/connections/secrets.rs](../screenroom-backend/src/connections/secrets.rs) | Managed private key-file publication and reads. |
| [src/jellyfin/connection.rs](src/jellyfin/connection.rs) | API-key connection and authenticated probe inputs. |
| [src/jellyfin/secret.rs](src/jellyfin/secret.rs) | Private key-file reader. |
| [src/jellyfin/revision.rs](src/jellyfin/revision.rs) | Adapter locator digest, distinct from managed connection revisions. |
| [src/jellyfin/network.rs](src/jellyfin/network.rs) | Destination validation, bounded DNS, and pinned transport addresses. |
| [src/catalog_delta.rs](src/catalog_delta.rs) | Provider-neutral delta capability and fallback contracts. |
| [src/jellyfin/delta.rs](src/jellyfin/delta.rs) | Jellyfin timestamp traversal. |
| [src/user_state.rs](src/user_state.rs) | Personal state contract, separate from shared metadata. |

# Boundaries

Provider adapters own bounded upstream I/O and typed provider results. Backend repositories own catalog transactions and source admission.
The shared client never calls Jellyfin directly.

# Contracts

- Keep tokens, token-file paths, private revisions, and upstream error bodies out of public DTOs and diagnostics.
- Adapter locator digests exclude key contents, filesystem metadata, symlink targets, and remote server state.
- Managed revisions bind connection identity, credential reference, authenticated server identity, and normalized URL; source snapshots also bind connection generation.
- Runtime connection checks separately verify ServerId; neither digest authenticates a dataset or proves credential freshness.
- Preserve response limits, request timeouts, cancellation, and classified retry inputs.
- Keep personal watched state separate from catalog metadata.
- Keep timestamp polling overlap, durable checkpoints, capability checks, and complete-reconcile fallback coupled.
- Treat LibraryChanged as optional future hints, not a durable journal.
- Runtime mock catalogs are not supported; isolated test doubles do not authorize product fallbacks.
- Web setup uses managed backend connections through the shared operator capability. First-add supports no edit, rotation, or replacement API.
- Apply [destination-security invariants](../../.agents/skills/changing-providers/SKILL.md#destination-security) to every provider connection path.

# Dependents

- [Worker](../screenroom-index-worker/src/) consumes provider pages, artwork, and delta capabilities.
- [User service](../screenroom-user/src/) consumes source discovery, profile selection, and personal state.

# Hints

- Use `changing-providers` for transport, credentials, source revision, or profile changes.
