# Component outline

This outline distinguishes the implemented mock API and client core from intended service scope.
The [client API guide](client-api.md) owns the detailed mock contract and development procedures.

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
The client contract requires catalog data, media artwork, provider branding, and notices from the mock API through the core.
Libraries, provider presence and status, search capabilities, rights, item colors, and rating providers and scales follow server records.
Shells must not substitute client fixtures, per-title branches, fake account flows, or fabricated refresh outcomes.
Application icons, system glyphs, themes, and generic image placeholders remain UI assets, not catalog content.
The app guides in the [documentation index](README.md) describe each shell's integration and prerequisites.

## Implemented mock boundary

| Path | Component |
| --- | --- |
| `crates/screenroom-mock-server/` | Development server with Browse, Catalog, Search, and Preferences unary services, global CatalogInfo, artwork HTTP routes, and health route. |
| `proto/screenroom/client/v1/client.proto` | Implemented `screenroom.client.v1` wire contract. |
| `fixtures/catalog/` | Canonical neutral catalog fixture, artwork and provider-brand bytes, notices, and provenance records; development server inputs, not client resources. |

The server uses fixture data and process-memory preferences per anonymous mock profile. It does not use PostgreSQL.
The mock boundary provides no authentication, provider connections, sync, playback, or real account persistence.
CatalogInfo exposes the complete artwork manifest and ordered notices, independent of Home results or previously visited items.
The [client API guide](client-api.md#artwork-and-provenance) defines image transport, attribution, and fixture-tool boundaries.

## Services and worker

These entries describe intended scope; the crates remain scaffolds outside the mock boundary.

| Path | Component |
| --- | --- |
| `crates/screenroom-gateway/` | Rust user/gateway service: user association with playback adapters where supported; Jellyfin only for now. |
| `crates/screenroom-catalog/` | Rust catalog service: a copy of the index in our database, embeddings, and links between items in other catalogs. |
| `crates/screenroom-index-worker/` | Rust index worker: embeddings, text search, and catalog sync from adapters. |

## Adapters

These entries describe intended integrations, not active provider connections.

| Path | Component |
| --- | --- |
| `crates/screenroom-catalog-adapter/` | Rust catalog adapter: Jellyfin, IMDb, TVDB, TMDB, Letterboxd, and Plex. |
| `crates/screenroom-playback-adapter/` | Rust playback adapter: Netflix, Amazon, HBO, Jellyfin, Plex, and others. |
| `crates/screenroom-remote-adapter/` | Rust remote adapter: remote actions on a particular item. |

A custom remote adapter could turn on a TV, launch an app, then play an item.

## Shared infrastructure

| Path | Component |
| --- | --- |
| `crates/screenroom-proto/`, `proto/`, `buf.yaml`, `buf.gen.yaml` | Protobuf library with Buf generation and Tonic; includes the implemented client mock contract. |
| `crates/screenroom-observability/` | Scaffold for an OpenTelemetry and profiling library. |
| `compose.yaml` | Local PostgreSQL configuration, independent of the mock server. |

Protobuf generation and observability use Waltr as a pattern reference. Screenroom remains standalone.
