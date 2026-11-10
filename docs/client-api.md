# Client API and core

Screenroom's client boundary supports a persistent local backend with offline typed provider adapters, local user selectors, and device-local settings.
It provides no authentication, live provider connections, or playback.
The wire contract separates catalog and user ownership:

| Package | Schema and ownership |
| --- | --- |
| `screenroom.catalog.v1` | [Catalog schema](../proto/screenroom/catalog/v1/catalog.proto): BrowseService, CatalogService, SearchService, and catalog messages. |
| `screenroom.user.v1` | [User schema](../proto/screenroom/user/v1/user.proto): UserService.GetCurrentUser and local user records. |
| `screenroom.common.v1` | [Common schema](../proto/screenroom/common/v1/common.proto): shared SearchTypes and SearchMethods. |
| `screenroom.playback_gateway.v1` | [Empty playback gateway package](../proto/screenroom/playback_gateway/v1/playback_gateway.proto): no messages or RPC services. |

This wire namespace cutover breaks old clients. The server provides no legacy namespace aliases.
Native UniFFI and WASM transport retain their roles; immutable configuration now uses `user_id`, not a mock profile.
The intended backend [public deployment contract](components.md#intended-public-deployment) does not add authentication to the local stack.

## Responsibilities

| Component | Owns |
| --- | --- |
| [Rust client core](../crates/screenroom-client-core/src/lib.rs) | Domain records, RPC transport, deadlines, errors, request deduplication, cancellation, read caches, and artwork bytes. |
| Native shells | Native layout, navigation, focus, view state, and image decode; Swift and Kotlin consume UniFFI, while Rust uses the crate directly. |
| Web SPA | Browser UI and image decode; consumes the same Rust core through WASM. |
| [Backend](backend.md) | Persistent catalog, local users, scoped search, bounded catalog information, paginated artwork credits, and same-origin artwork routes. |
| [Canonical fixtures](../fixtures/catalog/) | Neutral catalog data, source records, artwork and provider-brand bytes, notices, and rights provenance. |

Native core transport uses Tonic gRPC. WASM uses `tonic-web-wasm-client` through browser fetch.
The shared Envoy edge routes both protocols and artwork through one public origin.
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

The local API provides no account workflow. Shells must not simulate authentication through Welcome, Continue, or logout screens.
Refresh invalidates core reads and requests fresh data. Shells report actual request outcomes, not synthetic counters or library-sync success.

## Use the local backend

With explicit local lifecycle approval, start the persistent instance from the repository root:

```sh
bash scripts/backend.sh up
bash scripts/backend.sh wait
```

The [backend guide](backend.md#persistent-local-instance) defines dependencies, lifecycle ownership, model configuration, and non-destructive restart behavior.
Clients default to `http://127.0.0.1:50051`. Public artwork URLs must use that same configured origin.
Standalone client suites attach to the persistent descriptor; they do not start a substitute API server.
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
| `user_id` | Empty for anonymous catalog reads, otherwise a trusted-local UUID selector. |
| `timeout_ms` | Required deadline from 1 through 120000 milliseconds per network operation. |

The core sends a nonempty `user_id` as `x-screenroom-user-id` on RPCs.
It selects an enabled local user; it is not authentication, a token, or a security boundary.
Construct another client when endpoint or user changes. Configuration remains immutable.
Native and WASM bindings expose only public catalog/common/user contracts, not private operator/internal bindings or backend SQLx dependencies.

`Item` carries hierarchy, metadata, tags, colors, artwork, ratings, runtime, provider references, rights, and optional fixture progress.
`Item.provenance_notes` contains ordered strings for generic display; `metadata_retrieved_at` preserves the metadata snapshot timestamp.
Clients display these values without title-specific interpretations or a substitute client timestamp.
`HomeRow` contains ordered items; `MediaLibrary` exposes a library identifier and title.
`ItemPage` and `CreditPage` expose results plus `next_page_token`.
`Rights`, `Artwork`, `Rating`, `Runtime`, and `ProviderReference` preserve source and display details instead of UI-only text.
Unknown rating vote counts remain absent (`null` in WASM), not zero.
`Runtime` preserves optional exact seconds, minimum/maximum seconds, and version-specific notes and display text.
The model generator maps wire `Library` to domain `MediaLibrary`.

`CatalogInfo` contains bounded `notices`, bounded `branding`, and `retrieved_at` in Rust.
`CatalogNotice` contains six string fields: `id`, `section`, `title`, `text`, `link_label`, and `link_url`.
The server supplies notice order and the `metadata`, `catalog`, `artwork`, and `rights` sections.
Clients preserve notice text, order, and links. Empty titles and links do not require substitute copy.
CatalogInfo returns at most 32 notices and eight branding records, not the complete artwork inventory.
Its timestamp uses the newest valid eligible persisted provider snapshot in UTC; unknown timestamps remain empty, never the request clock.
`list_artwork(scope, page)` supplies complete enabled-scope credits, including unvisited portraits, character stills, and logos.
Clients enumerate those pages on demand; they do not preload all image bytes or infer completeness from visited titles.
`get_catalog_info()` follows the same read-cache, deduplication, deadline, cancellation, and invalidation rules as other catalog reads.

## Unary operations

BrowseService, CatalogService, and SearchService use `screenroom.catalog.v1`; UserService uses `screenroom.user.v1`.
For example, RPC paths include `/screenroom.catalog.v1.BrowseService/GetHome` and `/screenroom.user.v1.UserService/GetCurrentUser`.
Envoy routes both public packages through the same local endpoint.

| Service / RPC | Core method | Result and behavior |
| --- | --- | --- |
| `BrowseService/GetHome` | `get_home(scope)` | Scoped rows; mapped local users receive typed mock Continue Watching state. |
| `BrowseService/ListLibraries` | `list_libraries(source_id, page)` | Paginated stable library identifiers and titles. |
| `BrowseService/ListLibraryItems` | `list_library_items(scope, page)` | Scoped top-level library items. |
| `CatalogService/GetCatalogInfo` | `get_catalog_info()` | Bounded ordered notices, branding, and persisted snapshot timestamp. |
| `CatalogService/GetItem` | `get_item(id, scope)` | One scoped item; unknown or ineligible identifiers return `NotFound`. |
| `CatalogService/ListChildren` | `list_children(parent_id, page, scope)` | Scoped direct children; an empty result is valid. |
| `CatalogService/ListCredits` | `list_credits(item_id, page, scope)` | Scoped complete role credits; an empty result is valid. |
| `CatalogService/ListArtwork` | `list_artwork(scope, page)` | Complete paginated artwork credits, independent of visited titles. |
| `SearchService/Search` | `search(request)` | SearchPage with items, continuation, effective methods, and explicit degradation. |
| `SearchService/GetSearchCapabilities` | `get_search_capabilities()` | Supported methods, types, sources, semantic limits, and query length. |
| `UserService/GetCurrentUser` | `get_current_user()` | Enabled local user selected by explicit UUID metadata. |

`SourceLibraryScope` contains `source_id` and `library_id`; empty fields mean all enabled installation sources and libraries.
The clients preserve stable `movies` and `tv` library IDs and pass scope through details, children, credits, search, and continuation pages.
Provider choices `all`, `tmdb`, and `jellyfin` remain separate from installation source keys such as `mock-jellyfin` and `mock-tmdb`.
Public item kinds remain `Movie`, `Series`, `Season`, and `Episode`; search excludes seasons.

`Page.size=0` selects 50 results; the maximum is 200. Start with an empty token.
Treat page tokens as opaque values. Reuse each token only with its original query or collection.
An empty `next_page_token` marks the last page. Invalid tokens return `InvalidArgument`.
Tokens bind the operation, target, normalized scope, and query where applicable. They do not establish user ownership.
Search retains a short-lived ordered result snapshot and rechecks eligibility on continuation.

### Search limits

Search supports `Movie`, `Series`, and `Episode` types, plus `text`, `fuzzy`, `semantic`, and `title_only` controls.
At least one type and one of text, fuzzy, or semantic must remain active.
Title-only restricts text to titles, disables semantic, and requires text or fuzzy.
Text uses PostgreSQL full-text search; fuzzy uses `pg_trgm` title similarity.
Semantic uses exact 1024-dimensional pgvector cosine retrieval across all eligible source documents, not a lexical shortlist or tag approximation.
Fixed `k=60` reciprocal-rank fusion precedes best-source-document canonical deduplication; display metadata follows separate precedence.
No approximate nearest-neighbor index or reranker participates.
The server normalizes whitespace and accepts at most 8000 normalized characters.
A blank query returns `InvalidArgument`, unlike the retired mock contract. Clients clear blank searches locally.
Absent types default to movie, series, and episode. An explicit type selection must remain nonempty.

Sources are `all`, `tmdb`, and `jellyfin`.
Provider filters select eligible persisted documents; queries do not crawl TMDB or Jellyfin.
Clients populate choices from `get_search_capabilities()`, not a duplicate local list.
Clients also use its `semantic_description` and `maximum_query_length` instead of client-authored capability claims.
Search requests use explicit filters, not server preferences. Shells pass local settings in `SearchRequest`.
If semantic lacks availability, compatibility, or complete coverage, a valid lexical fallback returns `effective_methods.semantic=false` and `degradation_reason`.
The reasons are `embedding_unavailable`, `embedding_incompatible`, and `embedding_coverage_incomplete`.
A semantic-only request fails when no valid lexical fallback exists. Clients display degradation rather than a false semantic-success claim.

### Local settings

The backend has no PreferencesService. `ClientSettings`, `default_settings`, and `validate_settings` define the shared local settings contract.
Version-1 platform stores persist scope, provider, types, methods, and subtitle preference:

- Apple uses Foundation atomic JSON in Application Support, with errors and a save-retry action.
- Apple migrates legacy UserDefaults only when no settings file exists.
- Android validates SharedPreferences JSON and uses synchronous `commit` to detect write failure.
- Linux validates private XDG JSON and uses atomic file replacement.
- Web validates localStorage under an origin/endpoint/user/namespace key and retains choices in memory after write failure.

Storage failures produce a warning, not a claim that persistence succeeded. Local settings do not confer backend identity or access rights.

Core defaults select movie, series, and episode, text/fuzzy/semantic methods, all-library scope, provider `all`, and subtitle preference `off`.
Web controls initially select Movies and Shows. Title-only defaults to false and disables semantic.
Subtitle values are `off`, `library_tracks`, and `transcription`.
These values record a preference only; they provide no subtitle retrieval or transcription service.

## Errors, cancellation, and caches

The native API exposes typed `ClientError` variants.
WASM rejects promises with an `Error` that carries the corresponding snake_case `code`.

| Native error | Meaning |
| --- | --- |
| `InvalidArgument` | Invalid configuration, request, filter, user selector, or page token. |
| `NotFound` | Unknown item or artwork HTTP 404. |
| `Unavailable` | Transport failure or other unsuccessful artwork HTTP status. |
| `DeadlineExceeded` | The configured operation deadline expires. |
| `Cancelled` | Core cancellation or a request generation change invalidates the result. |
| `ResourceExhausted` | Core concurrency/artwork limits or bounded server capacity. |
| `Rpc` | Other gRPC statuses; preserves the numeric status code and reason. |
| `InvalidResponse` | Invalid model data, absent required response data, or unsupported artwork MIME type. |

WASM `Rpc` failures also expose `grpc_code`. The core performs no automatic retries.
Uncached reads fail when the server is unreachable; the core does not substitute bundled fixture data.
Shells own error presentation, empty states, and explicit retry actions.

Each client deduplicates equivalent active reads and permits at most 32 distinct active operations.
`cancel_all()` aborts active shared operations and rejects stale results. It preserves completed caches.
`invalidate_reads()` cancels active operations and clears completed read results; artwork remains cached.
A refresh calls `invalidate_reads()` before fresh reads. It does not trigger library sync.
Refresh errors remain errors; a client must not report success from an optimistic local state change.

Completed reads use an insertion-order memory cache with at most 64 entries and 2 MiB of serialized keys and values.
The cache has no TTL or disk persistence. Failed results do not enter the completed-read cache.
Artwork has a separate insertion-order memory cache: at most 32 entries and 8 MiB total.

## Artwork and provenance

The backend serves verified private Garage bytes at `/artwork/{uuid}/revisions/{positive_revision}/{native|original}`.
It publishes absolute same-origin URLs in item records, credit records, branding, and paginated artwork credits.
The core's `load_artwork(url)` accepts only `/artwork/` URLs on the configured endpoint origin.
It rejects URL credentials, queries, and fragments.
It returns raw bytes; each renderer owns image decode and placeholders.
Absent or failed images use truthful unavailable states, initials, or abstract placeholders, never client-local fixture artwork.
The core accepts JPEG, PNG, and SVG MIME types and limits each response to 4 MiB.
Native and browser artwork requests reject redirects. Clients never fetch S3 or provider artwork directly.
The backend preserves complete published revisions during replacement; unknown or disabled images return 404 and storage failures return 503.

The [normalized fixture](../fixtures/catalog/fixture.json) supplies API items, credits, and home rows.
[Title records](../fixtures/catalog/titles.json), [artwork records](../fixtures/catalog/artwork.json), and the [TMDB snapshot](../fixtures/catalog/tmdb.json) retain provenance.
The [notice fixture](../fixtures/catalog/notices.json) owns catalog-wide legal, provider, and fixture-specific attribution copy.
Clients render those notices through CatalogInfo, not duplicate legal paragraphs or branches for specific titles.
CatalogInfo branding owns provider-brand discovery; clients must not fabricate artwork records or logo URLs.
The offline adapter supplies the approved TMDB logo from `tmdb.png` for native decoders.
Canonical fixtures retain both that raster copy and the original SVG with provenance; no bundled-client exception applies.
Title rights, metadata rights, artwork licenses, and provider references describe distinct concerns.
A public-domain title does not imply public-domain metadata, artwork, restorations, or scores.
Preserve attribution, source links, license details, and jurisdiction caveats in client detail views.
The fixture includes Creative Commons artwork and the TMDB logo under distinct terms; it is not uniformly public domain.
The server notices carry the required TMDB attribution statement.

### Canonical fixture tools

The [fixture generator](../scripts/generate-mock-fixture.py) requires Python 3.
It combines canonical title records, the TMDB snapshot, artwork records, and notices into `fixtures/catalog/fixture.json`.
Typed offline adapters consume canonical fixture inputs; runtime clients never read those files.
After canonical input edits, regenerate the normalized fixture from the repository root:

```sh
python3 scripts/generate-mock-fixture.py
```

Fixture regeneration does not update persisted catalog state automatically.
An approved ingest through the current backend must apply fixture changes; ordinary startup does not reset operator state or reseed volumes.

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

The default public endpoint uses loopback and cleartext HTTP. The local stack provides no authentication or per-user catalog ACL.
Do not expose this stack to an untrusted network.
Use platform-specific development exceptions only for local backend access; these exceptions do not define a production transport policy.
Browser CORS permits only the configured origins, GET/POST/OPTIONS, and explicit gRPC-Web headers.
CORS does not protect native clients or make local UUID selectors private.
Private RPC network separation is not authentication. The local stack has no production-exposure approval or account-isolation guarantee.
