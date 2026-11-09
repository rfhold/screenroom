# Client API and core

Screenroom's mock boundary supports client development without real accounts, provider connections, library sync, playback, or a database.
The protocol package is `screenroom.client.v1`; its [protobuf schema](../proto/screenroom/client/v1/client.proto) defines the wire contract.

## Responsibilities

| Component | Owns |
| --- | --- |
| [Rust client core](../crates/screenroom-client-core/src/lib.rs) | Domain records, RPC transport, deadlines, errors, request deduplication, cancellation, read caches, and artwork bytes. |
| Native shells | Native layout, navigation, focus, view state, and image decode; Swift and Kotlin consume UniFFI, while Rust uses the crate directly. |
| Web SPA | Browser UI and image decode; consumes the same Rust core through WASM. |
| [Mock server](../crates/screenroom-mock-server/src/lib.rs) | Fixture catalog, global catalog information, ordered notices, search, mock preferences, and same-origin artwork HTTP routes. |
| [Canonical fixtures](../fixtures/catalog/) | Neutral catalog data, source records, artwork and provider-brand bytes, notices, and rights provenance. |

Native core transport uses Tonic gRPC. WASM uses `tonic-web-wasm-client` through browser fetch.
The server accepts both protocols on one endpoint; it does not require a separate gRPC-Web proxy.
Prost wire types stay inside the core. Shells consume the [domain records](../crates/screenroom-client-core/src/models.rs).
The [WASM adapter](../crates/screenroom-client-core/src/wasm.rs) exposes async methods and [TypeScript models](../crates/screenroom-client-core/src/wasm-models.ts).

This contract does not assert that every shell implements every operation.
Use the [Apple](../apps/apple/README.md), [Android](../apps/android/README.md), [Linux](../apps/linux/README.md), and [web](../apps/web/README.md) guides for shell-specific instructions.

### Server-driven client contract

All catalog content, media artwork, provider branding, and attribution notices reach shells through the Rust core.
Shells must not package catalog JSON, media artwork, or provider logos as runtime resources.
Shells must not call provider APIs or fetch artwork outside the core.
Raw source snapshots and fixture tools belong to the development server boundary, not runtime clients.

Shells use server records for libraries, provider presence and status, search capabilities, rights, item colors, and rating providers and scales.
They render generic records rather than fixture identifiers, title-specific branches, or client-authored provider and legal notices.
Application icons, system glyphs, themes, protocol-control labels, and generic image placeholders remain UI concerns.

The mock API provides no account workflow. Shells must not simulate authentication through Welcome, Continue, or logout screens.
Refresh invalidates core reads and requests fresh data. Shells report actual request outcomes, not synthetic counters or library-sync success.

## Run the mock server

From the repository root, start the development server:

```sh
bash scripts/mock-server.sh
```

The [helper](../scripts/mock-server.sh) builds the locked Rust package and runs `target/debug/screenroom-mock-server`.
It requires Cargo and a compatible Rust toolchain. No database or provider credential is necessary.

In another terminal, check the HTTP health route:

```sh
curl --fail http://127.0.0.1:50051/health
```

The route returns `screenroom-mock-server` plus a newline. Stop the server with Ctrl-C.

| Environment variable | Default | Meaning |
| --- | --- | --- |
| `SCREENROOM_MOCK_LISTEN` | `127.0.0.1:50051` | Socket address for the development server. |
| `SCREENROOM_MOCK_PUBLIC_BASE` | `http://` plus the listen address | Absolute origin that the server adds to artwork paths. |
| `SCREENROOM_MOCK_CORS_ORIGINS` | `http://localhost:5173,http://127.0.0.1:5173,http://localhost:4173,http://127.0.0.1:4173` | Comma-separated browser origin allowlist. |

Public base and CORS values require explicit HTTP(S) origins without credentials, paths, queries, fragments, or a trailing slash.
Set the public base to the exact origin that clients use for the endpoint; the core rejects artwork from another origin.
Loopback addresses refer to the client device itself. Emulator, container, and physical-device routes require their own reachable address.
The app guides own those routes and platform cleartext exceptions.

## Build the core

The helper supports macOS and Linux hosts; Apple builds require macOS, and Android builds require macOS or x86-64 Linux.
The [build helper](../scripts/build-client-core.sh) accepts these modes from the repository root:

| Command | Output and prerequisites |
| --- | --- |
| `bash scripts/build-client-core.sh native` | Host Rust library under `target/debug/`; requires Cargo. |
| `bash scripts/build-client-core.sh bindings` | Swift and Kotlin UniFFI output under `target/client-core/bindings/`; requires Python 3 for checked Swift postprocessing and reads a host `.dylib` on macOS or `.so` on Linux. |
| `bash scripts/build-client-core.sh wasm` | Browser module under `target/client-core/wasm/`; requires the `wasm32-unknown-unknown` target and compatible `wasm-bindgen`. |
| `bash scripts/build-client-core.sh apple` | `target/client-core/apple/ScreenroomClientCore.xcframework`; requires macOS, Xcode, Apple SDKs, and the selected Rust targets. |
| `bash scripts/build-client-core.sh android` | ARM64 and x86-64 libraries under `target/client-core/android/jniLibs/`; requires Android NDK and Rust targets. |

`SCREENROOM_CORE_PROFILE=release` selects release output; the default is `debug`.
`WASM_BINDGEN` overrides the WASM tool; otherwise the helper prefers `target/client-core/tools/bin/wasm-bindgen`, then `PATH`. The CLI version must match `Cargo.lock`; missing or mismatched versions fail before compilation with installation instructions.
`SCREENROOM_APPLE_TARGETS` selects Apple triples. `SCREENROOM_APPLE_BUILD_STD_TARGETS` selects triples that use nightly `-Zbuild-std`.
Treat build-std overrides as target/toolchain workarounds, not a general requirement for every Apple build.
The helper currently defaults that set to `x86_64-apple-tvos`; that path requires nightly and Rust standard-library sources.
The Android helper selects the NDK host directory `darwin-x86_64` on macOS or `linux-x86_64` on Linux.
It rejects other Linux architectures and defaults to NDK `29.0.14206865`.
`ANDROID_NDK_HOME` overrides the NDK directory; `ANDROID_HOME` or `ANDROID_SDK_ROOT` selects the SDK root.
The default SDK root is `~/Library/Android/sdk` on macOS or `~/Android/Sdk` on Linux.
The shell guides define their packaging steps and toolchain requirements.

## Configuration and models

Create one core `Client` with an explicit `ClientConfig`:

| Field | Contract |
| --- | --- |
| `endpoint` | HTTP(S) origin without credentials, path, query, fragment, or trailing slash; manual loopback default: `http://127.0.0.1:50051`. |
| `mock_profile` | Required opaque value: 1–128 ASCII letters, digits, `-`, `_`, or `.`. |
| `timeout_ms` | Required deadline from 1 through 120000 milliseconds per network operation. |

The core sends `mock_profile` as `x-screenroom-mock-profile` on every RPC.
The value selects anonymous mock state; it is not an identity, token, account, or security boundary.

`Item` carries hierarchy, metadata, tags, colors, artwork, ratings, runtime, provider references, rights, and optional fixture progress.
`Item.provenance_notes` contains ordered strings for generic display; `metadata_retrieved_at` preserves the metadata snapshot timestamp.
Clients display these values without title-specific interpretations or a substitute client timestamp.
`HomeRow` contains ordered items; `MediaLibrary` exposes a library identifier and title.
`ItemPage` and `CreditPage` expose results plus `next_page_token`.
`Rights`, `Artwork`, `Rating`, `Runtime`, and `ProviderReference` preserve source and display details instead of UI-only text.
Unknown rating vote counts remain absent (`null` in WASM), not zero.
`Runtime` preserves optional exact seconds, minimum/maximum seconds, and version-specific notes and display text.
The model generator maps wire `Library` to domain `MediaLibrary`.

`CatalogInfo` contains `artwork: Vec<Artwork>`, `notices: Vec<CatalogNotice>`, and `retrieved_at: String` in Rust.
`CatalogNotice` contains six string fields: `id`, `section`, `title`, `text`, `link_label`, and `link_url`.
The server supplies notice order and the `metadata`, `catalog`, `artwork`, and `rights` sections.
Clients preserve notice text, order, and links. Empty titles and links do not require substitute copy.
CatalogInfo covers the entire catalog, not Home results, the selected library, or items that a user already visited.
Its artwork list includes portraits, character stills, and provider logos, even when no visible item references them.
`get_catalog_info()` follows the same read-cache, deduplication, deadline, cancellation, and invalidation rules as other catalog reads.

## Unary operations

All RPC service names below use the `screenroom.client.v1` package.

| Service / RPC | Core method | Result and behavior |
| --- | --- | --- |
| `BrowseService/GetHome` | `get_home(library_id)` | Home rows filtered by library; progress values represent fixtures, not playback state. |
| `BrowseService/ListLibraries` | `list_libraries()` | Server-defined library identifiers and titles; the fixture supplies `all`, `movies`, and `tv`. |
| `BrowseService/ListLibraryItems` | `list_library_items(library_id, page)` | Top-level library items. |
| `CatalogService/GetCatalogInfo` | `get_catalog_info()` | Complete artwork manifest, ordered catalog notices, and snapshot timestamp; response field: `info`. |
| `CatalogService/GetItem` | `get_item(id)` | One item; unknown identifiers return `NotFound`. |
| `CatalogService/ListChildren` | `list_children(parent_id, page)` | Direct children of a known item; an empty result is valid. |
| `CatalogService/ListCredits` | `list_credits(item_id, page)` | Credits for a known item; an empty result is valid. |
| `SearchService/Search` | `search(request)` | Fixture results filtered by query, library, type, methods, and source. |
| `SearchService/GetSearchCapabilities` | `get_search_capabilities()` | Supported methods, types, sources, semantic limits, and query length. |
| `PreferencesService/GetPreferences` | `get_preferences()` | Current mock-profile preferences or defaults. |
| `PreferencesService/UpdatePreferences` | `update_preferences(preferences, update_mask)` | Validated field-mask merge; returns the complete preferences. |

`Page.size=0` selects 40 results; the maximum is 100. Start with an empty token.
Treat page tokens as opaque values. Reuse each token only with its original query or collection.
An empty `next_page_token` marks the last page. Invalid libraries and invalid tokens return `InvalidArgument`.

### Search limits

Search supports `Movie`, `Series`, and `Episode` types, plus `text`, `fuzzy`, `semantic`, and `title_only` controls.
At least one type and one of text, fuzzy, or semantic must remain active.
Title-only restricts text to the title and disables the tag-based semantic branch.
Text search uses substring comparison; fuzzy search uses bounded edit-distance checks against titles and title words.
The semantic option approximates relevance through ASCII tag matches. It does not use embeddings or an external model.
The server normalizes ASCII text and uses at most 160 normalized characters. Raw queries over 1024 bytes return `InvalidArgument`.
An empty normalized query returns no items.

Sources are `all`, `tmdb`, and `jellyfin`.
`tmdb` filters fixture provider references; it does not call TMDB. `jellyfin` returns no results because no provider connection exists.
These values describe the mock implementation; clients populate choices from `get_search_capabilities()`, not a duplicate local list.
Clients also use its `semantic_description` and `maximum_query_length` instead of client-authored capability claims.
Search requests use their explicit filters. Absent type/method records use server defaults, not the profile's saved preferences.
Shells must read saved preferences and pass the desired filters in `SearchRequest`.

### Mock preferences

The server holds preferences in process memory per mock profile. Clients that share a profile share that state.
A server restart discards all preferences; a client restart alone does not discard server state.
The server accepts at most 256 profiles with saved preferences.

Defaults select movies and series, text/fuzzy/semantic methods, library `all`, source `all`, and subtitle preference `off`.
Episodes and title-only default to false.
The required, nonempty update mask accepts only `search_types`, `search_methods`, `library_id`, `source`, and `subtitle_preference`.
Each mask path replaces its whole field; nested mask paths have no support.
Subtitle values are `off`, `library_tracks`, and `transcription`.
These values record a preference only; they provide no subtitle retrieval or transcription service.

## Errors, cancellation, and caches

The native API exposes typed `ClientError` variants.
WASM rejects promises with an `Error` that carries the corresponding snake_case `code`.

| Native error | Meaning |
| --- | --- |
| `InvalidArgument` | Invalid configuration, request, filter, profile, page token, or field mask. |
| `NotFound` | Unknown item or artwork HTTP 404. |
| `Unavailable` | Transport failure or other unsuccessful artwork HTTP status. |
| `DeadlineExceeded` | The configured operation deadline expires. |
| `Cancelled` | Core cancellation or a request generation change invalidates the result. |
| `ResourceExhausted` | Core concurrency/artwork limits or the server's saved-profile limit. |
| `Rpc` | Other gRPC statuses; preserves the numeric status code and reason. |
| `InvalidResponse` | Invalid model data, absent required response data, or unsupported artwork MIME type. |

WASM `Rpc` failures also expose `grpc_code`. The core performs no automatic retries.
Uncached reads fail when the server is unreachable; the core does not substitute bundled fixture data.
Shells own error presentation, empty states, and explicit retry actions.

Each client deduplicates equivalent active reads and permits at most 32 distinct active operations.
Preference writes use unique operation keys; the core does not deduplicate writes.
`cancel_all()` aborts active shared operations and rejects stale results. It preserves completed caches.
Cancellation does not guarantee that the server rolls back a preference write.
`invalidate_reads()` cancels active operations and clears completed read results; artwork remains cached.
A refresh calls `invalidate_reads()` before fresh reads. It does not trigger library sync.
Refresh errors remain errors; a client must not report success from an optimistic local state change.

Completed reads use an insertion-order memory cache with at most 64 entries and 2 MiB of serialized keys and values.
The cache has no TTL or disk persistence. Failed results do not enter the completed-read cache.
Preference reads bypass that cache so clients can observe another client's writes to the same profile.
A successful preference write clears completed reads.
Artwork has a separate insertion-order memory cache: at most 32 entries and 8 MiB total.

## Artwork and provenance

The server serves compiled fixture bytes at `/artwork/{file}`.
It publishes absolute artwork URLs in item records, credit records, and the complete CatalogInfo manifest.
The core's `load_artwork(url)` accepts only `/artwork/` URLs on the configured endpoint origin.
It rejects URL credentials, queries, and fragments.
It returns raw bytes; each renderer owns image decode and placeholders.
Absent or failed images use truthful unavailable states, initials, or abstract placeholders, never client-local fixture artwork.
The core accepts JPEG, PNG, and SVG MIME types and limits each response to 4 MiB.
Native HTTP disables redirects. Browser fetch retains browser redirect behavior; this mock server serves direct artwork responses.

The [normalized fixture](../fixtures/catalog/fixture.json) supplies API items, credits, and home rows.
[Title records](../fixtures/catalog/titles.json), [artwork records](../fixtures/catalog/artwork.json), and the [TMDB snapshot](../fixtures/catalog/tmdb.json) retain provenance.
The [notice fixture](../fixtures/catalog/notices.json) owns catalog-wide legal, provider, and fixture-specific attribution copy.
Clients render those notices through CatalogInfo, not duplicate legal paragraphs or branches for specific titles.
The artwork manifest owns provider-brand discovery; clients must not fabricate artwork records or logo URLs.
The mock exposes the approved TMDB logo as `tmdb.png` for native decoders.
Canonical fixtures retain both that raster copy and the original SVG with provenance; no bundled-client exception applies.
Title rights, metadata rights, artwork licenses, and provider references describe distinct concerns.
A public-domain title does not imply public-domain metadata, artwork, restorations, or scores.
Preserve attribution, source links, license details, and jurisdiction caveats in client detail views.
The fixture includes Creative Commons artwork and the TMDB logo under distinct terms; it is not uniformly public domain.
The server notices carry the required TMDB attribution statement.

### Canonical fixture tools

The [fixture generator](../scripts/generate-mock-fixture.py) requires Python 3.
It combines canonical title records, the TMDB snapshot, artwork records, and notices into `fixtures/catalog/fixture.json`.
The mock server embeds that normalized catalog fixture, not the raw JSON inputs.
After canonical input edits, regenerate the normalized fixture from the repository root:

```sh
python3 scripts/generate-mock-fixture.py
```

Then stop the current mock server. Rebuild and restart it with `bash scripts/mock-server.sh`; that helper builds the locked server package.
A server rebuild alone does not incorporate raw input edits into the normalized fixture.

The root [TMDB importer](../scripts/import-tmdb.mjs) writes only the canonical snapshot at `fixtures/catalog/tmdb.json`.
It requires `TMDB_READ_ACCESS_TOKEN` and performs external provider requests.
An import requires separate, explicit approval for that provider operation; ordinary local client work does not authorize it.
The importer and raw snapshots supply development server inputs, not a runtime client network bypass.
The [importer tests](../scripts/import-tmdb.test.mjs) use stub responses without a provider import:

```sh
node --test scripts/import-tmdb.test.mjs
```

### Client-boundary checks

From the repository root:

```sh
pnpm check:client-boundary
pnpm test:client-boundary
```

The [static guard](../scripts/check-client-boundary.mjs) scans production client source, resources, and selected build configuration across all four client families.
It rejects known local-artwork, fixture, provenance, and fake-flow patterns; tests and the separate website remain outside its scope.
The [guard tests](../scripts/test-client-boundary.mjs) exercise those detection rules.
This guard does not prove complete data flow or inspect runtime network traffic and packaged artifacts.
Platform integration tests and resource audits remain necessary.

## Development security boundary

The default endpoint uses loopback and cleartext HTTP. The server provides no TLS listener, authentication, or authorization.
Do not expose this server to an untrusted network.
Use platform-specific development exceptions only for local mock access; these exceptions do not define a production transport policy.
Browser CORS permits only the configured origins, GET/POST/OPTIONS, and explicit gRPC-Web headers.
CORS does not protect native clients or make mock profiles private.
The mock API has no production compatibility or account-isolation guarantee.
