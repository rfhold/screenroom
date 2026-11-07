# Component outline

This outline records the initial component scope, not implemented functionality.

## Clients and website

| Path | Component |
| --- | --- |
| `website/` | Static Cloudflare Pages website for downloads, docs, and about; not an SPA. |
| `apps/apple/` | Native Apple TV, iOS, and macOS shells. |
| `apps/android/` | Native Android and Android TV shells. |
| `apps/linux/` | Native Linux shell. |
| `apps/web/` | Svelte SPA shell with pnpm and TypeScript. |
| `crates/screenroom-client-core/` | Rust client core. |

## Services and worker

| Path | Component |
| --- | --- |
| `crates/screenroom-gateway/` | Rust user/gateway service: user association with playback adapters where supported; Jellyfin only for now. |
| `crates/screenroom-catalog/` | Rust catalog service: a copy of the index in our database, embeddings, and links between items in other catalogs. |
| `crates/screenroom-index-worker/` | Rust index worker: embeddings, text search, and catalog sync from adapters. |

## Adapters

| Path | Component |
| --- | --- |
| `crates/screenroom-catalog-adapter/` | Rust catalog adapter: Jellyfin, IMDb, TVDB, TMDB, Letterboxd, and Plex. |
| `crates/screenroom-playback-adapter/` | Rust playback adapter: Netflix, Amazon, HBO, Jellyfin, Plex, and others. |
| `crates/screenroom-remote-adapter/` | Rust remote adapter: remote actions on a particular item. |

A custom remote adapter could turn on a TV, launch an app, then play an item.

## Shared infrastructure

| Path | Component |
| --- | --- |
| `crates/screenroom-proto/`, `proto/`, `buf.yaml`, `buf.gen.yaml` | Protobuf library with Buf generation and Tonic. |
| `crates/screenroom-observability/` | OpenTelemetry and profiling library. |
| `compose.yaml` | PostgreSQL database. |

Protobuf generation and observability use Waltr as a pattern reference. Screenroom remains standalone.
