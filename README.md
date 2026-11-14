# Screenroom

Domain: **screenroom.tv**.

Screenroom pairs native clients and a Web SPA with a shared Rust client core and a persistent local backend.
PostgreSQL stores the shared catalog, local users, provider connections, queue progress, search documents, and embeddings.
Garage stores private artwork bytes. Jellyfin supplies real provider data through backend adapters.
Runtime product mocks, TMDB fixture feeds, and demo-user bootstrap are not supported.
Authentication and playback remain outside the implementation.

Clients receive catalog content, artwork, provider branding, and notices through the backend and shared core.
They own presentation and local preferences, not provider credentials or direct Jellyfin HTTP.
The website remains separate from the Web SPA; historical showcase images are static illustrations, not provider-integration evidence.

## Start here

| Document | Covers |
| --- | --- |
| [Documentation index](docs/README.md) | Product, architecture, operations, and platform guides. |
| [Backend development](docs/backend.md) | Persistent local lifecycle, migrations, identity, search, artwork, and telemetry. |
| [Jellyfin connections](docs/jellyfin.md) | Implemented managed backend setup, private API-key ownership, and limits. |
| [Client API and core](docs/client-api.md) | Public reads, separate operator capability, bindings, and toolchains. |
| [Indexing architecture](docs/indexing.md) | Shared queue, policies, retry/redrive, durable progress, and incremental acquisition. |
| [Index maintenance](docs/index-maintenance.md) | Canonical repair, publication, and trusted-local operator security. |
| [Testing](docs/testing.md) | Isolated fault tests, real-source acceptance, E2E commands, and platform limits. |
| [Component outline](docs/components.md) | Implemented boundaries and intended product scope. |

The default local instance uses `http://127.0.0.1:50051`; an empty catalog is healthy before source setup.
The operator API requires explicit opt-in and grants trusted-local capability, not authenticated administrator access.
Web Maintenance supports Jellyfin URL/API-key discovery, profile/library selection, Save, and Save and refresh through the shared operator capability.
No native onboarding UI forms part of that target.

Lifecycle changes, provider acquisition, credential operations, and macOS GUI automation require separate approval.
This documentation grants no permission to start services, apply migrations, or expose the stack publicly.
