# Client API and core

Screenroom's client boundary supports a persistent local backend, typed provider adapters, local user selectors, and device-local settings.
Clients own no provider connections.
The backend supports managed Jellyfin connections and gateway playback for Web and native clients; it provides no authentication.
The wire contract separates service ownership:

| Package | Schema and ownership |
| --- | --- |
| `screenroom.catalog.v1` | [Catalog schema](../proto/screenroom/catalog/v1/catalog.proto): BrowseService, CatalogService, SearchService, and catalog messages. |
| `screenroom.user.v1` | [User schema](../proto/screenroom/user/v1/user.proto): UserService local user records and Jellyfin profile links. |
| `screenroom.common.v1` | [Common schema](../proto/screenroom/common/v1/common.proto): shared SearchTypes and SearchMethods. |
| `screenroom.playback_gateway.v1` | [Playback schema](../proto/screenroom/playback_gateway/v1/playback_gateway.proto): PlaybackService sessions, events, tracks, and media descriptors. |

This wire namespace cutover breaks old clients. The server provides no legacy namespace aliases.
Native UniFFI and WASM transport retain their roles; immutable configuration now uses `user_id`, not a mock profile.
The intended backend [public deployment contract](components.md#intended-public-deployment) does not add authentication to the local stack.

## Responsibilities

| Component | Owns |
| --- | --- |
| [Rust client core](../crates/screenroom-client-core/src/lib.rs) | Domain records, RPC transport, deadlines, errors, request deduplication, cancellation, read caches, and artwork bytes. |
| Native shells | Native layout, navigation, focus, view state, and image decode; Swift and Kotlin consume UniFFI, while Rust uses the crate directly. |
| Web SPA | Browser UI and image decode; consumes the same Rust core through WASM. |
| [Backend](backend.md) | Persistent catalog, local users, scoped search, artwork, and gateway-owned playback sessions/media. |

Native core transport uses Tonic gRPC. WASM uses `tonic-web-wasm-client` through browser fetch.
The shared Envoy edge routes both protocols, artwork, and playback media through one public origin.
Prost wire types stay inside the core. Shells consume the [domain records](../crates/screenroom-client-core/src/models.rs).
The [WASM adapter](../crates/screenroom-client-core/src/wasm.rs) exposes async methods and [TypeScript models](../crates/screenroom-client-core/src/wasm-models.ts).

This contract does not assert that every shell implements every operation.
Use the [Apple](../apps/apple/README.md), [Android](../apps/android/README.md), [Linux](../apps/linux/README.md), and [web](../apps/web/README.md) guides for shell-specific instructions.

### Server-driven client contract

Catalog content and media artwork reach shells through the Rust core.
The core also exposes backend branding and notices. If shells consume them, those records come from the backend through the core.
This origin contract requires no global provider-credit UI or unused branding and notice requests.
Shells must not package catalog JSON, media artwork, or provider logos as runtime resources.
Shells must not call provider APIs or fetch artwork outside the core.
Playback control uses the core; video, HLS, and subtitles use validated gateway URLs outside gRPC and WASM.
Runtime product fixtures and mock adapters are not supported server inputs either.

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
Native apps and standalone Web builds default to `https://preview-screenroom.holdenitdown.net`. Deployed Web builds use their own origin.
To use the local instance, set the platform's endpoint override. Use `http://127.0.0.1:50051` where the platform allows loopback HTTP; Apple and Android release builds require HTTPS.
Public artwork URLs must use the configured API origin.
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
| `endpoint` | Required HTTP(S) origin without credentials, path, query, fragment, or trailing slash. Native HTTPS RPCs verify the server certificate against WebPKI roots. |
| `user_id` | Empty for anonymous catalog reads, otherwise a trusted-local UUID selector. |
| `timeout_ms` | Required deadline from 1 through 120000 milliseconds per network operation. |

The core sends a nonempty `user_id` as `x-screenroom-user-id` on RPCs.
It selects an enabled local user; it is not authentication, a token, or a security boundary.
Construct another client when endpoint or user changes. Configuration remains immutable.
The shared `Client` exposes catalog, common, user, and playback operations. It does not expose internal RPC bindings or backend SQLx dependencies.
The bindings also expose the separate, explicit [operator capability](#operator-capability).

`Item` carries hierarchy, metadata, tags, colors, artwork, ratings, runtime, provider references, rights, and optional personal progress.

### Watched state

`Item.watched` is an optional boolean: `Option<bool>` in Rust, nullable native bindings, and `boolean | null` in WASM.
The wire field is `optional bool watched = 21`.

| Value | Meaning |
| --- | --- |
| `true` | The selected source explicitly reports watched. |
| `false` | The selected source explicitly reports unwatched. |
| Absent / `null` | Watched state is unknown. |

Progress remains independent. A progress value of 100 does not imply watched, and absent progress does not imply unwatched.
Catalog reads hydrate watched state from persisted snapshots for the selected local user and each item's chosen source item.
They do not merge watched observations across providers or request provider state for this overlay.
Anonymous reads, absent observations, expired snapshots, and invalid user/source links return unknown.
The [backend acquisition boundary](backend.md#watched-snapshots) defines snapshot publication and expiry.
The separate Home Continue Watching path can still read provider state; this overlay does not make every Home operation provider-independent.

Jellyfin supplies explicit `Played` values for movies and episodes. Series and seasons remain unknown; clients do not infer container completion.
Web, Apple, Android, and Linux posters display a top-right checkmark only for `true`, with the accessible label `Watched`.
Neither `false` nor unknown displays a checkmark. Progress bars remain independent.
The field adds no manual watched controls.
Playback reports write personal provider state; pending or ambiguous effects keep cached watched status unknown.

### Playback methods

The ordinary `Client` exposes these methods on Rust, native UniFFI, and WASM:

| Method | Request / result |
| --- | --- |
| `create_playback_session` | `CreatePlaybackSessionRequest` / `CreatePlaybackSessionResponse`. |
| `get_playback_session` | `GetPlaybackSessionRequest` / `GetPlaybackSessionResponse`. |
| `update_playback_session` | `UpdatePlaybackSessionRequest` / `UpdatePlaybackSessionResponse`. |
| `report_playback_event` | `ReportPlaybackEventRequest` / `ReportPlaybackEventResponse`. |
| `stop_playback_session` | `StopPlaybackSessionRequest` / `StopPlaybackSessionResponse`. |

The [playback contract](playback.md) owns states, sequences, revisions, receipts, personal identity, and HTTP media rules.
These methods require an enabled local-user selector; session operations also require an in-memory control capability.
They do not use `OperatorClient` or direct provider HTTP.
Preparation uses 30 seconds; other playback calls use 10 seconds, independent of the Web metadata default of three seconds.
Playback calls serialize through the mutation gate and retain no completed-read cache.
The core never retries mutations automatically.
Explicit same-session retries retain the exact mutation UUID and body in memory; reload supplies no journal recovery.
Ambiguous reports require a session-state check, not automatic replay.
Cancellation ends local waits; it does not prove that a provider mutation stopped.
Profile mutation cancels stale playback requests; player lifecycle hooks also stop local media.
Web routes preserve explicit source/library read scope. Player navigation uses a one-use in-memory launch intent; deep entry, reload, and Forward require explicit Play.
Catalog/search history snapshots are bounded, memory-only, and cleared on profile changes; they contain no playback authority.

Public positions, durations, and offsets use decimal millisecond strings in Rust, UniFFI, WASM, and TypeScript.
Nullable fields retain `Option<T>` / `T | null`; sequences and revisions use bounded `uint32` values.
The core validates media URLs against the configured API origin, session, revision, resource UUID, and separate media capability.
Media bytes never enter the core, its artwork cache, or a WASM response buffer.
Web converts timeline strings to video seconds only after checked range and round-trip validation.
Native source integrates AVPlayer/AVKit on Apple, Media3 on Android mobile/TV, and embedded GStreamer on Linux.
The [native player contract](playback.md#native-players) owns controls, restoration, stop differences, plugin dependencies, and seek limits.
Shared bindings alone do not establish player integration or native application acceptance.

### Metadata and artwork

Episode artwork in catalog reads uses the season poster, then the series poster from the episode's selected source. Only complete published posters qualify. Without either poster, artwork is absent and clients show their normal placeholder. `ListChildren` for a season is the exception: its episode grid keeps each episode's own artwork. This selection happens on reads and needs no metadata update, reindex, or provider request.

Episode `Item.subtitle` reads project `Series title · S02E05` from the selected source's episode `metadata.index`, parent season `metadata.index`, and canonical parent series title. Ancestors must be visible in the requested source, library, and provider scope; kind filters apply only to the target episode. Index zero is valid for specials. If either index or the series title is absent, reads return the stored subtitle unchanged, even when empty. Direct episode-to-series parents also retain the stored subtitle because no season index exists. Reads preserve non-episode subtitles. This bounded batch projection makes no metadata writes, search index changes, or provider requests.
Apple, Android, and Linux episode cards show the series name beside a small muted number such as `S05:E09`.
The episode title appears below. Explicit season grids shorten the number to `E09` only when the episode's parent matches that season.
Unknown episode numbers stay absent; shells do not invent hierarchy from navigation history.
`Item.provenance_notes` contains ordered strings for generic display; `metadata_retrieved_at` preserves the metadata snapshot timestamp.
Backend and core records retain these values; client detail views omit provenance notes and per-item legal panels.
`HomeRow` contains ordered items; `MediaLibrary` exposes a library identifier and title.
`ItemPage` and `CreditPage` expose results plus `next_page_token`.
`Rights`, `Artwork`, `Rating`, `Runtime`, and `ProviderReference` preserve source and display details instead of UI-only text.
Unknown rating vote counts remain absent (`null` in WASM), not zero.
`Runtime` preserves optional exact seconds, minimum/maximum seconds, and version-specific notes and display text.
The model generator maps wire `Library` to domain `MediaLibrary`.

`CatalogInfo` contains bounded `notices`, bounded `branding`, and `retrieved_at` in Rust.
`CatalogNotice` contains six string fields: `id`, `section`, `title`, `text`, `link_label`, and `link_url`.
The server supplies notice order and the `metadata`, `catalog`, `artwork`, and `rights` sections.
For notices they display, clients preserve server text, order, and links. Empty titles and links do not require substitute copy.
CatalogInfo returns at most 32 notices and eight branding records, not the complete artwork inventory.
Its timestamp uses the newest valid eligible persisted provider snapshot in UTC; unknown timestamps remain empty, never the request clock.
`list_artwork(scope, page)` supplies complete enabled-scope credits, including unvisited portraits, character stills, and logos.
This API remains available; client shells do not enumerate its pages for a global credits inventory.
Visited titles do not establish inventory completeness.
`get_catalog_info()` uses the completed-read cache, active-read deduplication, deadlines, cancellation, and explicit invalidation.

## Operator capability

`Client.operator()` derives a typed `OperatorClient` from the same configured endpoint, selector, and transport.
The operator process serves its own protobuf service over gRPC and gRPC-Web.
Rust and native UniFFI expose the same records and operations as WASM.
The separate operator service is API-only; native clients have no operator onboarding UI.
This contract does not claim native UI parity for operator controls.
The [indexing architecture](indexing.md#canonical-operator-api) defines policy, queue status, and terminal redrive contracts.
Canonical indexing replaces older poster-maintenance operations.
Web Maintenance uses the managed-connection RPCs for [Jellyfin setup](jellyfin.md#web-setup).
Native apps expose no onboarding UI.
Generic operator records remain provider-neutral; browsers do not implement provider traversal, retry classification, or checkpoint rules.
The API supplies no administrator authentication.

| Configuration | Contract |
| --- | --- |
| `OperatorClientConfig.endpoint` | HTTP(S) origin, with no credentials, path, query, fragment, or trailing slash. Use the same origin as `ClientConfig`. |
| `OperatorClientConfig.timeout_ms` | Required operation deadline from 1 through 120000 milliseconds. |

Use the shared client capability to reuse its configured endpoint and user selector.
`configuration()` returns immutable configuration; `configuration_namespace()` returns the endpoint namespace, not backend identity or authorization proof.
The [maintenance guide](index-maintenance.md#browser-boundary) owns the trusted-network boundary.

| Methods | Typed contract |
| --- | --- |
| `health`, `sources`, `libraries` | Availability and paginated source/library discovery. |
| `discover_connection` | `OperatorConnectionDiscovery`; transient authenticated probe from `OperatorDiscoverConnectionRequest`. |
| `save_connection` | `OperatorSavedConnection`; explicit `OperatorSaveConnectionRequest`, with optional initial refresh and exact policy version. |
| `connections` | `OperatorConnectionPage`; safe connection summaries, UUID cursor, and limit 1–200. |
| `cancel_all` | Abort active operations and reject stale-generation responses. |

### Canonical indexing methods

These methods use the same explicit `OperatorClient`, endpoint guards, cancellation, and mutation-ambiguity rules.
They do not add native UI or public catalog RPCs.

| Method | Typed result and contract |
| --- | --- |
| `indexing_runs` | `OperatorRunPage`; canonical source run history, without the removed `trigger` field. |
| `indexing_candidates` | `OperatorCandidatePage`; canonical per-run safe outcomes and optional `pending_only` filter. |
| `indexing_policies` | `OperatorIndexingPolicyPage`; canonical policies for one source. |
| `indexing_configure` | `OperatorIndexingPolicy`; accepts `OperatorIndexingConfigureRequest` with request UUID, optional version-checked update, and public configuration. |
| `indexing_progress` | `OperatorIndexingProgress` or null; durable policy version, watermark, and next reconciliation time. |
| `indexing_start` | `OperatorRun`, not `OperatorRunStatus`; accepts `OperatorStartRequest` with request UUID, policy version, and optional narrowed scope. |
| `indexing_run_status` | `OperatorRunStatus`; observations, terminal reason, coverage, permitted actions, enumeration, settlement, and success. |
| `indexing_execution` | `OperatorIndexingExecution` or null; actual mode, lower/candidate watermarks, fallback, and settlement time. |
| `indexing_pause` | `OperatorRunStatus`; explicit permitted pause. |
| `indexing_resume` | `OperatorRunStatus`; paused-run continuation, not failed-job redrive. |
| `indexing_cancel` | `OperatorRunStatus`; explicit permitted cancellation. |
| `indexing_jobs` | `OperatorJobPage`; sanitized jobs, current redrive eligibility, and safe ineligibility reason. |
| `indexing_attempts` | `OperatorAttemptPage`; job attempt history, fence cursor, and redrive generation. |
| `indexing_redrives` | `OperatorRedrivePage`; durable job redrive history with generation cursor. |
| `indexing_redrive` | `OperatorRedrive`; accepts `OperatorRedriveRequest` with explicit request UUID; no transport-level automatic retry. |
| `indexing_queue` | `OperatorQueueDiagnostics`; accepts nullable run/capability context and retains non-atomic observation semantics. |

`OperatorIndexingConfiguration` carries modes, components, strategies, scope, artwork reuse, effective retry, and per-work/run limits.
It has no `operator_profile_id`, provider locator, credential, or raw policy field.
The server resolves the enabled source's managed connection profile; an unavailable binding rejects configure.
Clients reject unsupported policy combinations before dispatch; the [backend policy contract](indexing.md#policy-model) remains authoritative.

Canonical policy/job/attempt/redrive collections accept 1–200; run/candidate history accepts 1–1,000.
Policies and jobs use UUID cursors; attempts use decimal fence cursors and redrives use decimal generation cursors.
Attempt fences and all `i64` budgets, versions, and counts use `bigint` in WASM; generation and attempt counts use bounded numbers.
`indexing_queue` preserves null capability context separately from an explicit empty list.
Its result preserves `atomic: false`, `queue_observation`, `blockers_as_of`, nullable eligibility counts, overlapping count semantics, and unknown worker liveness.

The protobuf service defines named methods, selectors, and pagination. Operation payloads preserve exact UTF-8 domain JSON inside the protobuf messages.
Wide integers never pass through JavaScript number serialization.
The response limit is 2 MiB; request payloads retain the 65,536-byte limit.
Backend and Rust method support do not establish complete generated-binding or Web UI parity for managed setup.
Neither surface authenticates an administrator.

### Transport and exact values

Every read requests fresh server state; the capability has no completed-read cache, public-client cache sharing, or automatic retries.
Native requests use the shared Tonic channel. WASM uses gRPC-Web with the same configured backend, omits credentials, and rejects redirects.
WASM aborts fetch on cancellation or deadline.
Requests allow at most 65,536 body bytes; core responses allow at most 2 MiB.
The core permits at most 64 active operations per operator client.
`OperatorPage` supplies `after` and `limit` from 1 through 1000; page records return `items` and `next_after`.

Operator `i64` fields preserve exact values in Rust/native records and use `bigint` in WASM.
These fields include policy versions, request budgets, and counters.
The Rust transport owns JSON serialization; shells must not coerce these values through JavaScript `number`.
Browser shells must preserve exact versions in memory; no operation journal supplies reload recovery.

`OperatorError` includes `Stale`, `SourceBusy`, `Invalid`, `NotFound`, `Forbidden`, `Unavailable`, `Cancelled`, `Deadline`, and `InvalidResponse`.
`Connection` carries a bounded `OperatorConnectionErrorCode` and mutation ambiguity, not raw provider diagnostics.
WASM errors carry snake_case `code`, `ambiguous`, and a safe blocker `run_id` for `source_busy`.
Transport, cancellation, deadline, and invalid-response failures mark uncertain mutation outcomes through `ambiguous`.
Errors expose no raw upstream messages, provider URLs, credentials, or SQL details.
Cancellation does not prove that a server mutation stopped.
Explicit same-session Retry retains the request UUID and exact body for idempotency; clients never retry mutations automatically.
An ambiguous mutation permits that exact retry or a current server-state check, not an unrelated replacement request.
Clients do not read older browser operation records; reload reads server state without journal replay, export, migration, or start continuation.
The [canonical API](indexing.md#canonical-operator-api) owns policy, idempotency, and run-control semantics.

Source contracts:

- [Operator client](../crates/screenroom-client-core/src/operator.rs): endpoint validation, requests, bounds, cancellation, and typed errors.
- [Operator records](../crates/screenroom-client-core/src/operator_models.rs): native/domain types.
- [WASM operator adapter](../crates/screenroom-client-core/src/operator_wasm.rs) and [TypeScript records](../crates/screenroom-client-core/src/operator-models.ts): browser methods and exact integer representation.

## Unary operations

BrowseService, CatalogService, and SearchService use `screenroom.catalog.v1`; UserService uses `screenroom.user.v1`.
For example, RPC paths include `/screenroom.catalog.v1.BrowseService/GetHome` and `/screenroom.user.v1.UserService/GetCurrentUser`.
Envoy routes both public packages through the same local endpoint.

| Service / RPC | Core method | Result and behavior |
| --- | --- | --- |
| `BrowseService/GetHome` | `get_home(scope)` | Scoped rows; mapped local users receive Continue Watching state, independent of watched snapshots. |
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
| `UserService/ListJellyfinSources` | `list_jellyfin_sources(page)` | Enabled Jellyfin source keys and labels, without upstream URLs or credentials. |
| `UserService/ListJellyfinProfiles` | `list_jellyfin_profiles(source_id, page)` | Eligible enabled upstream profile choices for one source. |
| `UserService/GetJellyfinLink` | `get_jellyfin_link(source_id)` | Persisted source/profile link for the selected local user, or absent. |
| `UserService/SetJellyfinLink` | `set_jellyfin_link(source_id, jellyfin_user_id)` | Persist a link to an eligible enabled profile for the selected local user. |
| `UserService/ClearJellyfinLink` | `clear_jellyfin_link(source_id)` | Idempotently unlink the selected local user from one source. |

`SourceLibraryScope` contains `source_id` and `library_id`; empty fields mean all enabled installation sources and libraries.
Clients preserve server-supplied library IDs and pass scope through details, children, credits, search, and continuation pages.
Library IDs are opaque. Literal values such as `movies` and `tv` imply no media type, icon, or navigation behavior.
Provider filters remain separate from installation source keys; a filter enum does not establish a runtime provider adapter.
Public item kinds remain `Movie`, `Series`, `Season`, and `Episode`; search excludes seasons.

`Page.size=0` selects 50 results; the maximum is 200. Start with an empty token.
Treat page tokens as opaque values. Reuse each token only with its original query or collection.
An empty `next_page_token` marks the last page. Invalid tokens return `InvalidArgument`.
Tokens bind the operation, target, normalized scope, and query where applicable. They do not establish user ownership.
Search retains a short-lived ordered result snapshot and rechecks eligibility on continuation.

Apple, Android, and Linux library, children, and search views request an initial page of at most 100 items.
Their Load more controls append API-order results, suppress duplicate stable IDs, and show loaded counts rather than catalog totals.
Continuation failures preserve loaded cards and the retry token. Cancellation and request identity guards reject stale results.
Shells reject repeated continuation tokens and enforce bounded page traversal.

### Jellyfin profile links

Web, Apple, Android, and Linux expose Jellyfin source choices, eligible profile choices, the current link, Save, and Unlink.
These controls change a server-persisted progress mapping for the selected local UUID, not authentication or provider credentials.
Each Save or Unlink action issues one mutation, invalidates reads, and rereads the link.
An ambiguous failure triggers a read of server state, not an automatic mutation retry.
The profile Retry action reads server state only. Unknown state disables mutation controls until a read confirms the current link.
Profile discovery uses its own opaque page tokens, bound to the operation, selected local user, and source where applicable.
The public profile controls do not expose private operator mapping APIs.
Continue Watching failures show a generic explanation while other Home rows remain usable.

### Search limits

Search supports `Movie`, `Series`, and `Episode` types, plus `text`, `fuzzy`, `semantic`, and `title_only` controls.
At least one type and one of text, fuzzy, or semantic must remain active.
Title-only restricts text to titles, disables semantic, and requires text or fuzzy.
Text uses PostgreSQL full-text search; fuzzy uses `pg_trgm` title similarity.
Semantic uses exact 1024-dimensional pgvector cosine retrieval across all eligible source documents, not a lexical shortlist or tag approximation.
Fixed `k=60` reciprocal-rank fusion precedes best-source-document canonical deduplication; display metadata follows separate precedence.
No approximate nearest-neighbor index or reranker participates.
The server normalizes whitespace and accepts at most 8000 normalized characters.
A blank query returns `InvalidArgument`. Clients clear blank searches locally.
Absent types default to movie, series, and episode. An explicit type selection must remain nonempty.

Search sources are `all` and `jellyfin`.
Provider filters select eligible persisted documents; queries do not contact providers.
Source-supplied TMDB external IDs and item links remain metadata, not a search capability.
Clients populate choices from `get_search_capabilities()`, not a duplicate local list.
Clients also use its `semantic_description` and `maximum_query_length` instead of client-authored capability claims.
Search requests use explicit filters, not server preferences. Shells pass local settings in `SearchRequest`.
If semantic lacks availability, compatibility, or complete coverage, a valid lexical fallback returns `effective_methods.semantic=false` and `degradation_reason`.
The reasons are `embedding_unavailable`, `embedding_incompatible`, and `embedding_coverage_incomplete`.
A semantic-only request fails when no valid lexical fallback exists. Clients display degradation rather than a false semantic-success claim.

### Local settings

The backend has no PreferencesService. `ClientSettings`, `default_settings`, and `validate_settings` define the shared local settings contract.
`ClientSettings` contains four fields: `scope`, `source`, `types`, and `methods`.
Version-1 platform stores persist scope, provider, types, and methods:

- Apple uses Foundation atomic JSON in Application Support, with errors and a save-retry action.
- Apple migrates legacy UserDefaults only when no settings file exists.
- Android validates SharedPreferences JSON and uses synchronous `commit` to detect write failure.
- Linux validates private XDG JSON and uses atomic file replacement.
- Web validates localStorage under an origin/endpoint/user/namespace key and retains choices in memory after write failure.

Storage failures produce a warning, not a claim that persistence succeeded. Local settings do not confer backend identity or access rights.

Core defaults select movie, series, and episode, text/fuzzy/semantic methods, all-library scope, and provider `all`.
Web controls initially select Movies and Shows. Title-only defaults to false and disables semantic.
The shared load normalizer maps an old saved `tmdb` provider preference to `all` and preserves unrelated valid preferences.
All four clients use `normalize_saved_settings` on load; saves use strict validation and reject `tmdb`.
Ordinary settings files ignore the obsolete subtitle field. The active `ClientSettings` model and client controls contain no subtitle preference.
These settings supply no global subtitle preference or transcription service.
Per-session subtitle choices belong to the [players](playback.md), not saved catalog settings.

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
| `Playback` | Typed safe `PlaybackErrorCode` from the gateway; no upstream body. |
| `PlaybackUncertain` | A dispatched mutation loses a definitive transport, deadline, or cancellation outcome. |

WASM `Rpc` failures also expose `grpc_code`. The core performs no automatic retries.
WASM playback failures use `code=playback` with `playback_code`, or `code=playback_uncertain` for ambiguity.
Uncached reads fail when the server is unreachable; the core does not substitute bundled fixture data.
Shells own error presentation, empty states, and explicit retry actions.

Each client deduplicates equivalent active reads and permits at most 32 distinct active operations.
`cancel_all()` aborts active shared operations and rejects stale results. It preserves completed caches.
`invalidate_reads()` cancels active operations and clears completed read results; artwork remains cached.
A refresh calls `invalidate_reads()` before fresh reads. It does not trigger library sync.
Refresh errors remain errors; a client must not report success from an optimistic local state change.

Home, library-item, item, children, and search reads do not retain completed results, including continuation pages.
Repeated reads through the same client revalidate watched expiry and external item identity against the backend.
Equivalent active reads still share one operation, with the same cancellation and stale-result guards.
Other completed reads use an insertion-order memory cache with at most 64 entries and 2 MiB of serialized keys and values.
That cache has no TTL or disk persistence. Failed results do not enter the completed-read cache.
Artwork has a separate insertion-order memory cache: at most 32 entries and 8 MiB total.
Profile-link mutations invalidate core reads. Snapshot acquisition does not push updates into rendered screens; a refresh or another read obtains current state.

## Artwork and provenance

The backend serves verified private Garage bytes at `/artwork/{uuid}/revisions/{positive_revision}/{native|original}`.
It publishes absolute same-origin URLs in item records, credit records, branding, and paginated artwork credits.
The core's `load_artwork(url)` accepts only `/artwork/` URLs on the configured endpoint origin.
It rejects URL credentials, queries, and fragments.
It returns raw bytes; each renderer owns image decode and placeholders.
Apple and Android limit shell artwork work to eight active requests and share equivalent URL reads.
Cancellation detaches individual consumers and preserves reads that other consumers still need.
Linux uses a separate bounded artwork path; it does not share the Apple or Android scheduler.
Absent or failed images use truthful unavailable states, initials, or abstract placeholders, never client-local fixture artwork.
The core accepts JPEG, PNG, and SVG MIME types and limits each response to 4 MiB.
Native and browser artwork requests reject redirects. Clients never fetch S3 or provider artwork directly.
The backend preserves complete published revisions during replacement; unknown or disabled images return 404 and storage failures return 503.

Persisted provider records supply API items, credits, home rows, provenance, and rights data.
CatalogInfo owns bounded server notices and branding; clients do not invent provider-specific legal text.
Web, Linux, Apple, and Android omit global attribution and technical-credit footers and the complete artwork inventory.
They issue no footer-only CatalogInfo, branding, or artwork requests and retain no footer-only state, retries, or styles.
No provider-credit UI forms part of the client contract.
Clients must not invent legal paragraphs or add branches for specific titles.
The public CatalogInfo and ListArtwork APIs remain unchanged; the backend retains provenance, licenses, and attribution data.
CatalogInfo branding owns provider-brand discovery; clients must not fabricate artwork records or logo URLs.
Title rights, metadata rights, artwork licenses, and provider references describe distinct concerns.
A public-domain title does not imply public-domain metadata, artwork, restorations, or scores.
Web, Apple, Android, and Linux omit per-item rights, provenance, artwork license, creator, crop, and legal panels.
They retain per-item source identifiers, external links, ratings, snapshot disclosures, provider status, and people labels.
Backend and core records retain provenance and rights data.
Historical records retain their applicable attribution and license terms; no blanket public-domain assumption applies.
Product fixture generators, TMDB importers, and fixture-backed bootstrap are not supported interfaces.
The [testing guide](testing.md) distinguishes isolated synthetic responses from real-source acceptance.

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

## Client telemetry

Shipped native apps initialize OpenTelemetry once per process, not once per client or profile. Release builds default to `production`; set the app build environment explicitly when shipping a preview build. The label describes the app build, not its configured backend. Debug apps and tests that use only the library do not start native exporters.

On Web, deployment runtime configuration enables Faro. Home, catalog, details, and playback creation pass explicit W3C context through WASM, without a global fetch patch. The server uses that context as the parent of its RPC span. Native RPCs attach context through the shared client. Callers do not need to add headers to media URLs.

## Development security boundary

The default public endpoint uses loopback and cleartext HTTP. The local stack provides no authentication or per-user catalog ACL.
Do not expose this stack to an untrusted network.
Use platform-specific development exceptions only for local backend access; these exceptions do not define a production transport policy.
Browser CORS permits only configured origins and explicit route-specific methods/headers.
Playback media additionally permits HEAD and Range/If-Range; its anonymous requests carry no ordinary user-selector header.
CORS does not protect native clients or make local UUID selectors private.
Private RPC network separation is not authentication. The local stack has no production-exposure approval or account-isolation guarantee.
