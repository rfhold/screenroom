# Component outline

This outline owns intended product scope and distinguishes implemented local behavior from future integrations.
The [client API guide](client-api.md) owns the public client contract; the [backend guide](backend.md) owns local operational details.
The [indexing architecture](indexing.md) owns shared-queue and incremental behavior, supported backend policies, and explicit client and activation limits.

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
When shells consume catalog data, media artwork, provider branding, or notices, those records come from the backend through the core.
This origin contract requires no unused global branding or notice requests.
Libraries, provider presence and status, search capabilities, rights, item colors, and rating providers and scales follow server records.
Shells must not substitute client fixtures, per-title branches, fake account flows, or fabricated refresh outcomes.
Application icons, system glyphs, themes, and generic image placeholders remain UI assets, not catalog content.
Web, Linux, Apple, and Android omit global attribution and technical-credit footers, footer-only requests, and the complete artwork inventory.
The [client contract](client-api.md#artwork-and-provenance) requires no provider-credit UI.
All four client families omit per-item rights, provenance, artwork license, creator, crop, and legal panels.
Per-item source identifiers, external links, ratings, snapshot disclosures, provider status, and people labels remain available; backend provenance remains unchanged.
Apple, Android, and Linux expose explicit watched indicators, incremental library/children/search pages, and server-persisted Jellyfin profile links.
The [client contract](client-api.md#jellyfin-profile-links) defines profile failure handling and the trusted-local selector boundary, not authentication.
The app guides in the [documentation index](README.md) describe each shell's integration and prerequisites.

## Implemented local boundary

| Path | Component |
| --- | --- |
| `crates/screenroom-backend/` | Shared PostgreSQL repositories, startup migrations, durable jobs, embeddings, and private object storage. |
| `crates/screenroom-queue/` | Provider-neutral PostgreSQL transport, leases, fences, classified retry, attempt history, and terminal redrive. |
| `crates/screenroom-catalog-adapter/` | Real Jellyfin acquisition, authenticated connection probes, destination guards, hierarchy, artwork, and incremental capability fallback. |
| `proto/screenroom/catalog/v1/catalog.proto` | `screenroom.catalog.v1`: BrowseService, CatalogService, SearchService, and catalog messages. |
| `proto/screenroom/user/v1/user.proto` | `screenroom.user.v1`: local user reads and validated Jellyfin profile links for a trusted-local UUID selector. |
| `proto/screenroom/common/v1/common.proto` | `screenroom.common.v1`: shared SearchTypes and SearchMethods. |

PostgreSQL persists shared catalog data, local users, source links, durable ingest state, search documents, and embeddings.
Private managed connections bind Jellyfin server identity, selected libraries, and the installation profile.
Clients persist versioned settings locally; the backend provides no preference service.
CatalogInfo returns bounded notices and branding. ListArtwork supplies complete paginated credits independently of visited items.
Both APIs remain available unchanged; removal of global client credits UI does not remove backend provenance or license data.
An empty installation is healthy before source setup; startup performs no fixture import or provider acquisition.
Runtime product mocks, TMDB feeds, demo-user bootstrap, and manual provider profiles are not supported.
Isolated fault servers remain test support, not product adapters.
Authentication, playback, transcoding, downloads, and remote orchestration remain absent.
The [client API guide](client-api.md#artwork-and-provenance) defines image transport and attribution boundaries.
The [Jellyfin guide](jellyfin.md) owns managed setup, private credentials, and unsupported edit/rotation paths.

## Services and worker

The table describes intended business scope; implemented catalog and user reads use PostgreSQL through Tonic gRPC/gRPC-Web.
Playback gateway exposes a Tonic listener with empty routes, not a playback API.
The worker resolves managed Jellyfin connections dynamically; it provides no complete external-catalog guarantee.
The [backend development guide](backend.md) defines local startup and telemetry procedures.

| Path | Component |
| --- | --- |
| `crates/screenroom-user/` | Rust user service: user accounts and association with playback providers where supported; Jellyfin initially. |
| `crates/screenroom-catalog/` | Rust catalog service: a copy of the index in our database, embeddings, and links between items in other catalogs. |
| `crates/screenroom-playback-gateway/` | Rust playback gateway executable: provider integration for Netflix, Amazon, HBO, Jellyfin, Plex, and others. |
| `crates/screenroom-index-worker/` | Rust index worker: embeddings, text search, and catalog sync from adapters. |

### Indexing boundary

All indexing paths use one provider-neutral PostgreSQL queue under the [accepted architecture](indexing.md#queue-and-domain-responsibilities).
Stateless workers use durable progress, capability-aware claims, fences, classified retry, and explicit terminal redrive.
Domain repositories own source eligibility, catalog transactions, and publication.
Canonical backend policies support full, incremental, reconcile, and poster-only repair within their [validated combinations and limits](indexing.md#policy-model).
Incremental Jellyfin uses timestamp polls with overlap and periodic complete reconciliation; optional notification hints do not form a journal.
The canonical operator HTTP API exposes policies, progress, jobs, attempts, diagnostics, and terminal redrive.
Its separate service is API-only and also supports managed connection discovery/save.
Web Maintenance supports URL/API-key discovery, profile/library selection, Save, and Save and refresh through shared operator methods and guarded proxy routes.
Canonical Refresh, Pause, Resume, and Retry use saved policies; Advanced settings expose policy and queue details.
Browser operation journals and old maintenance routes have no compatibility requirement.
No native onboarding or operator UI forms part of this support.
Per-operation transport observers report consumed provider/embedding/storage bodies, not wire bytes or proof of index success.
Source support grants no live activation authority or collector-delivery guarantee.
The [maintenance guide](index-maintenance.md) preserves current poster behavior and the trusted-local operator boundary.

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

These entries describe intended integrations beyond the implemented real Jellyfin adapter.
The live adapter supports timestamp polling on admitted Jellyfin versions, not an authoritative deletion journal.

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
