# Component outline

This outline owns intended product scope and distinguishes implemented local behavior from future integrations.
The [client API guide](client-api.md) owns the public client contract; the [backend guide](backend.md) owns local operational details.

## Clients and website

| Path | Component |
| --- | --- |
| `website/` | Static Cloudflare Pages website for downloads, docs, and about; not an SPA. |
| `apps/apple/` | Native Apple TV, iOS, and macOS shells. |
| `apps/android/` | Native Android and Android TV shells. |
| `apps/linux/` | Native Linux shell. |
| `apps/web/` | Svelte SPA shell with pnpm and TypeScript. |
| `crates/screenroom-client-core/` | Implemented shared Rust client core: domain models, transport, errors, cancellation, and memory caches. |

Client shells own native layout, navigation, focus, and image decode. The web SPA uses the core through WASM.
The client contract requires catalog data, media artwork, provider branding, and notices from the backend through the core.
Libraries, provider presence and status, search capabilities, rights, item colors, and rating providers and scales follow server records.
Shells must not substitute client fixtures, per-title branches, fake account flows, or fabricated refresh outcomes.
Application icons, system glyphs, themes, and generic image placeholders remain UI assets, not catalog content.
The app guides in the [documentation index](README.md) describe each shell's integration and prerequisites.

## Implemented local boundary

| Path | Component |
| --- | --- |
| `crates/screenroom-backend/` | Shared PostgreSQL repositories, startup migrations, durable jobs, embeddings, and private object storage. |
| `crates/screenroom-catalog-adapter/` | Typed offline Jellyfin and TMDB adapters with explicit scope, hierarchy, progress, artwork, and provenance. |
| `proto/screenroom/catalog/v1/catalog.proto` | `screenroom.catalog.v1`: BrowseService, CatalogService, SearchService, and catalog messages. |
| `proto/screenroom/user/v1/user.proto` | `screenroom.user.v1`: UserService.GetCurrentUser for a trusted-local UUID selector. |
| `proto/screenroom/common/v1/common.proto` | `screenroom.common.v1`: shared SearchTypes and SearchMethods. |
| `fixtures/catalog/` | Canonical neutral fixture, artwork, notices, and provenance records; offline adapter inputs, not client resources. |

PostgreSQL persists shared catalog data, local users, source links, durable ingest state, search documents, and embeddings.
Typed mock adapters implement offline Jellyfin library traversal, bounded TMDB feeds, hierarchy enrichment, and mapped mock playback state.
Clients persist versioned settings locally; the backend provides no preference service.
CatalogInfo returns bounded notices and branding. ListArtwork supplies complete paginated credits independently of visited items.
The local stack provides no authentication, live provider crawl, playback, transcoding, downloads, or remote orchestration.
The [client API guide](client-api.md#artwork-and-provenance) defines image transport, attribution, and fixture-tool boundaries.

## Services and worker

The table describes intended business scope; implemented catalog and user reads use PostgreSQL through Tonic gRPC/gRPC-Web.
Playback gateway exposes a Tonic listener with empty routes, not a playback API.
The worker uses offline typed adapters, not live provider credentials or a complete external catalog.
The [backend development guide](backend.md) defines local startup and telemetry procedures.

| Path | Component |
| --- | --- |
| `crates/screenroom-user/` | Rust user service: user accounts and association with playback providers where supported; Jellyfin initially. |
| `crates/screenroom-catalog/` | Rust catalog service: a copy of the index in our database, embeddings, and links between items in other catalogs. |
| `crates/screenroom-playback-gateway/` | Rust playback gateway executable: provider integration for Netflix, Amazon, HBO, Jellyfin, Plex, and others. |
| `crates/screenroom-index-worker/` | Rust index worker: embeddings, text search, and catalog sync from adapters. |

### Intended public deployment

The catalog, user, and playback gateway services share one public DNS hostname.
Ingress routes requests to each service by fully qualified gRPC service/method path or HTTP path.
DNS resolves the shared hostname; it does not route request paths.
Each service independently validates authentication; ingress authentication alone does not establish trust.

These requirements describe architecture only, not an ingress installation or an authentication implementation.
The `screenroom.playback_gateway.v1` protobuf package remains empty, with no invented playback RPC contract.
The [implemented local boundary](#implemented-local-boundary) provides no authentication or permission for production exposure.
Envoy supplies local path routes on loopback; it does not satisfy the independent authentication requirement.

## Adapters

These entries describe intended integrations. The catalog adapter currently supplies typed offline Jellyfin and TMDB mocks, not active provider connections.

| Path | Component |
| --- | --- |
| `crates/screenroom-catalog-adapter/` | Rust catalog adapter: Jellyfin, IMDb, TVDB, TMDB, Letterboxd, and Plex. |
| `crates/screenroom-remote-adapter/` | Rust remote adapter: remote actions on a particular item. |

A custom remote adapter could turn on a TV, launch an app, then play an item.

## Shared infrastructure

| Path | Component |
| --- | --- |
| `crates/screenroom-proto/`, `proto/`, `buf.yaml`, `buf.gen.yaml` | Protobuf library with Buf generation and Tonic; public catalog, user, and common packages define the client contract. |
| `crates/screenroom-observability/` | Shared telemetry setup and optional profiling support; see the [backend development guide](backend.md). |
| `compose.yaml` | Persistent local PostgreSQL 18 with pgvector, Garage, Envoy, catalog, user, and worker; only the edge binds loopback. |

Screenroom remains standalone; shared telemetry uses community middleware and official OpenTelemetry SDK/exporter crates, not a Waltr runtime dependency.
