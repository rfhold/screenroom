# Playback contract

## Scope and source ownership

This guide describes the implemented Jellyfin VOD gateway and Web, Apple, Android, and Linux player source.
The [protobuf schema](../proto/screenroom/playback_gateway/v1/playback_gateway.proto) owns field names, enum values, and wire types.
Buf owns the Rust wire output.
The [model generator](../scripts/generate-client-models.py) owns ordinary Rust records and TypeScript models.

Source includes gateway control/media handlers, durable repositories, private provider transport, shared client methods, and platform controls.
Source support does not establish deployment readiness or live Jellyfin compatibility.
The [testing guide](testing.md#playback-checks) separates isolated checks from database, browser, and real-provider acceptance.
The [backend guide](backend.md#playback-gateway-runtime) owns configuration, private stores, readiness, and cleanup operations.

## Boundary and authority

The ordinary Rust/WASM `Client` controls playback through `screenroom.playback_gateway.v1.PlaybackService`.
Video, HLS, and subtitle requests use gateway HTTP media routes.
Media bytes never pass through the shared client core.
`OperatorClient` has no playback role.
Clients never contact Jellyfin directly.
Native players use the same gateway contract; platform engines and lifecycle differences appear [below](#native-players).

Every RPC requires `x-screenroom-user-id` for an enabled local user.
This UUID selects a user under the trusted-local contract; it does not authenticate the caller.
Every session operation except creation also requires the session UUID and its control capability.
Clients retain capabilities only in memory.
Clients never log capabilities or store them in platform settings, browser storage, navigation history, or durable replay journals.

Control and media capabilities remain separate.
Each capability contains 256 random bits, encoded as 64 lowercase hexadecimal characters.
A media capability authorizes only its session, transport revision, and approved resource scope.
A media capability never authorizes an RPC.
Public records expose no provider URL, filesystem path, provider token, or Jellyfin device/play-session identity.

## Operations

| RPC | Inputs | Result |
| --- | --- | --- |
| `CreatePlaybackSession` | Mutation UUID, public item ID, catalog scope, conservative client capabilities, start policy, optional position. | Authoritative session and control capability. |
| `GetPlaybackSession` | Session UUID and control capability. | Current state, accepted sequence, transport revision, pending operation, lease, and expiry. |
| `UpdatePlaybackSession` | Session authority, mutation UUID, expected revision, position, track choices, optional compatible-transcode fallback. | New transport or the identical operation's receipt. |
| `ReportPlaybackEvent` | Session authority, revision, event sequence/kind, position, pause/mute state, volume. | Accepted sequence, report outcome, authoritative session, renewed lease. |
| `StopPlaybackSession` | Session authority, mutation UUID, final sequence, revision, position, reason label. | Terminal session or explicit cleanup-pending state. |

Session and mutation IDs use UUIDs.
Receipts bind the local user, personal binding generation, operation, mutation UUID, and normalized input.
An identical operation reuses its receipt.
Changed input under the same mutation UUID conflicts.
The gateway stores capability hashes and protects any material necessary for receipt replay.

Item resolution requires an enabled source, current library membership, and a Movie or Episode.
The supplied catalog scope selects the exact source.
Ambiguous source matches return `playback_conflict` rather than choose another provider copy.
The session never silently changes sources.
Playback requires a current personal profile link; the installation's indexing profile never substitutes for that link.
Profile unlink/change, disabled users/sources, and connection-generation changes revoke affected sessions.

## Choices, precision, and nullability

`PlaybackStartPolicy` accepts `resume`, `beginning`, or `position`.
`position_ms` accompanies only `position`.
For `resume`, the gateway reads Jellyfin's current personal position.
Catalog percentages never supply seek positions.

Every public position, duration, and timeline offset uses a nonnegative decimal millisecond string.
The gateway validates decimal syntax and uses checked integer arithmetic for conversion to Jellyfin ticks.
Bindings preserve these strings exactly.
Web converts positions to video-element seconds only after checked range and round-trip validation.
Sequences and transport revisions use bounded `uint32`.
Lease and absolute-expiry fields use RFC 3339 UTC strings.
Volume uses a finite number within `[0, 1]`.

Protobuf message presence maps to Rust `Option<T>` and TypeScript `T | null`.
Optional scalar and enum fields preserve that same distinction.
Repeated records map to vectors and arrays.
Domain enums use lowercase snake-case JSON strings and closed TypeScript unions.
`unspecified` and unknown enum values never represent valid request choices.

`PlaybackClientCapabilities` contains conservative MP4, H.264, AAC, native HLS, MSE HLS, and WebVTT observations.
False means unsupported or unknown.
The backend owns the Jellyfin device profile; clients cannot supply arbitrary profile JSON.

Media-source and track IDs represent opaque, session-scoped choices.
Audio and subtitle IDs never represent unchecked provider stream indices.
Absent track choices retain the current choice.
`subtitles_off` explicitly disables subtitles and conflicts with `subtitle_track_id`.
Subtitle modes distinguish WebVTT, burn-in, and unavailable tracks.
Only available WebVTT tracks expose a gateway subtitle URL.

`PlaybackTransport` contains the method, MIME type, gateway media URL, revision, optional duration, start position, and timeline offset.
Methods distinguish direct play, direct stream/remux, and transcode.
The gateway prefers that order.
The gateway normalizes timelines once, including HLS and subtitle offsets.
A session permits at most one controlled compatible-transcode fallback, not an automatic retry loop.

## State, events, and uncertainty

Session states are `preparing`, `ready`, `playing`, `paused`, `stopping`, `stopped`, `expired`, `failed`, and `uncertain`.
The session omits its transport when preparation, revocation, or failure leaves no usable transport.
`cleanup_pending` distinguishes local termination from remote cleanup completion.
`pending_operation` identifies a mutation UUID or event sequence with a pending or uncertain outcome.

Report outcomes are `not_sent`, `pending`, `confirmed`, `uncertain`, and `failed`.
Events are `started`, `progress`, `paused`, `resumed`, `seeked`, and `heartbeat`.
Sequence order follows player events, not position order.
A backward seek remains valid.
Heartbeats renew the local lease without duplicate personal-state reports.
Transcode transports use the provider's narrow keepalive where necessary.

The gateway reserves durable event/operation state before provider I/O.
It accepts an identical event sequence/body once locally.
Changed-body reuse conflicts; older completed sequences do not repeat provider effects.
Stale transport revisions fail.
The gateway serializes each session's state mutations.
Each outbound provider dispatch rechecks session authority.
A profile change blocks later dispatch and stale results, but cannot retract a request that already left the process.

An ambiguous provider response remains `uncertain`.
Local deduplication does not prove provider exactly-once execution.
An uncertain start never triggers automatic replay because Jellyfin start can increment play count.
Later personal-state reports stop until resolution or termination.
A read-only reconciliation does not prove an exact play-count outcome.
After an ambiguous mutation, the client checks authoritative session state.
Cancellation ends local waits and media consumption; it does not prove that a provider mutation stopped.

Safe error codes match `PlaybackErrorCode` without its wire prefix.
They cover profile, source, item, media, authorization, conflict, expiry, capacity, and upstream failures.
RPC failures expose only a safe code, never an upstream error body.
The shared client maps these codes to typed failures.
Session `failure_code` describes a failure that remains available through session reads.

## Media and lifetime rules

The media prefix is `/playback/v1/media/`.
GET and HEAD support bounded streams, single byte ranges, and gateway-rewritten HLS resources.
Media capabilities expire with the lease and revoke on stop, profile change, or transport replacement.
Every resource and long stream requires current profile/source/session authority.
The gateway checks long streams at most five seconds apart.
Media query capabilities and child signatures require redaction, `Cache-Control: no-store`, and a no-referrer policy.
Media URLs contain bearer authority; copied URLs require the same care as credentials.

The gateway rejects external origins, traversal, unsupported encryption, and unknown playlist URI forms.
It maps validated playlist resources to opaque gateway URLs and accepts only bounded WebVTT subtitle responses.
The resource graph binds the exact item, media source, device, play-session identity, and transport revision.
PostgreSQL stores negotiated root and track resources, plus durable session and cleanup state.
Validated HLS children remain in the bounded process-local resource graph; playlist expansion creates no child database rows.
Root and track URLs retain their `cap` query field.
Child URLs add an opaque durable-root `anchor` and a `sig` field.
An independent private per-transport key signs the domain/version, session UUID, revision, durable-root UUID, and child UUID with HMAC-SHA256.
The signature contains no provider locator, path, or token.
Each child request requires its exact validated runtime reference and the durable root's current database media fence.
The same fence protects provider dispatch and long streams; signatures never extend the lease or replace profile/source checks.
Restart discards child references and keys; stop and transport replacement revoke old references through the durable fence.
The adapter preserves guarded DNS, pinned addresses, TLS identity, configured base paths, no-proxy behavior, and redirect rejection.
Binary media streams use backpressure and output chunks at most 256 KiB; no whole-film buffer exists in the core or WASM.
The adapter retains at most one upstream frame; the HTTP transport determines that frame's allocation size.
Single byte ranges preserve `206`, `416`, content length, MIME type, and allowed validators.
Multiple ranges fail explicitly; buffered playlist and subtitle ranges select bytes from the complete validated representation.
Buffered HEAD responses return the full representation length without a body; If-Range returns the full representation without a local validator.
Playlist bounds are 4 MiB per response, four nesting levels, and 8,192 total resources per transport, including root resources.
WebVTT responses have a 2 MiB bound; the gateway caps its rewritten playlist cache at 2 MiB per transport.
The gateway caches each rewritten playlist for its transport revision; it does not refresh manifests that change.
The adapter accepts valid master playlists and requires media playlists to contain segments and `ENDLIST`.
Unsupported non-ENDLIST, mixed, empty, or incomplete forms fail closed before resource mapping or caller-budget changes.
This implementation targets complete VOD manifests, not live TV.
Exact SPA-origin allowlists apply to control and media CORS.
Cross-origin media requests omit cookies and carry no ordinary local-user header.
Cookies supply no playback authority.
The Web video element sets its cross-origin mode before any media or subtitle URL.
Control preflight permits POST and explicit gRPC-Web/user-selector headers.
Media preflight permits GET, HEAD, OPTIONS, Range, and If-Range.
Media responses expose Content-Type, Content-Length, Content-Range, Accept-Ranges, ETag, and Last-Modified.

Default lease: 90 seconds; Web heartbeat: 15 seconds.
Default absolute lifetime: 12 hours; maximum paused interval: 30 minutes.
Media requests alone never extend the lease.
Default capacity: two active sessions per connection/profile and eight per installation.
A duplicate active connection/profile/item conflicts.
Capacity failures never silently stop another session.
Player exit or profile change stops playback.
A reload starts without playback authority.
Restart recovery performs cleanup, not playback replay.

Preparation has a separate bounded 30-second client deadline.
Control I/O and individual cleanup attempts have separate 10-second bounds.
Media uses header/idle deadlines, not a whole-film deadline.
These values describe source defaults, not client authority.
The executable uses fixed repository lifetime constants and default capacity limits; it exposes no environment overrides for them.

## Shared player presentation

This section defines the approved presentation contract for Web, Apple, Android, and Linux players.
It does not establish implementation parity or platform acceptance.
The [presentation checks](testing.md#player-presentation-checks) separate source and isolated checks from actual interaction.

- A black video stage fills the available viewport and preserves the media aspect ratio with contain sizing.
- Header, controls, status, and menus overlay the stage; their appearance never resizes the video.
- Small viewports retain usable controls without horizontal overflow.
- The header exposes back/close and a truncated title.
- The bottom overlay places the timeline above play/pause, backward/forward seek, mute/volume, tracks/settings, and available fullscreen controls.
- Web uses monochrome SVG icons; native players use corresponding vector symbols.
- Controls retain dynamic accessible labels, selected states, tooltips, and visible keyboard or TV focus.
- Desktop and touch targets measure at least 44 logical pixels; TV targets provide larger focus surfaces.
- Anchored menus float above the stage with bounded scrollable content, explicit subtitles Off, and discoverable session recovery.
- Click, touch, and focus open menus; hover alone never gates access.
- Back or Escape dismisses an open menu before player exit.
- Overlays fade after about three seconds of idle active playback.
- Pause, preparation, blocked/error states, scrubbing, open menus, and keyboard/TV control focus keep overlays visible.
- Initial programmatic or default TV focus does not prevent idle fade indefinitely.
- Before TV controls hide, focus moves safely to a stable reveal target.
- The first D-pad or Select input reveals controls and restores focus without playback actions.
- Human keyboard, menu, and scrub interactions preserve usable focus.
- Pointer motion, touch, or remote input reveals controls.
- Hidden overlays intercept no clicks and expose no invisible focus targets; a reveal gesture never changes playback accidentally.
- Subtitle safe areas preserve readable captions around visible overlays.
- Idle timers affect presentation only and end on teardown.

Menu, fullscreen, and overlay transitions preserve the media surface, playback owner, session, and report sequence.
This contract changes no gateway authority, mutation/retry semantics, logical seek rules, profile isolation, or platform lifecycle behavior.

## Web player lifecycle

Movie and Episode details expose **Play** with the current source/library scope.
Play grants a one-use in-memory launch intent and navigates to `/play/[itemId]`, which opens a viewport-sized player.
The [shared presentation contract](#shared-player-presentation) governs the stable video stage, icon overlays, menus, and idle behavior.
Loading and unavailable states show a concise message and relevant actions instead of an empty native video surface.
The modal blocks catalog interaction and background scroll.
**Back to details**, **Close player**, and Escape exit to the validated internal detail return URL.
Browser Back restores the prior view after metadata is ready.
The optional **Fullscreen** action uses the browser API; unsupported browsers expose a disabled unavailable action.
The route preserves source/library scope in query parameters.
Direct entry, reload, and Forward require explicit Play; none recreates a previous session.
Playback authority and mutation bodies never enter URLs, history state, or storage.
The player requests live personal resume; labelled custom controls operate the video element for play, pause, seek, mute, and volume.
Timeline labels use the logical session position, converting the transport offset once in each direction.
**Playback settings** contains track choices and expandable session information, including explicit checks and transport retries.
WebVTT cue placement reserves space above visible control overlays.
Audio choices renegotiate transport through the gateway.
Subtitle choices distinguish gateway WebVTT, provider burn-in, and unavailable tracks; Off explicitly disables subtitles.
Burn-in support does not imply complete ASS styling or universal subtitle compatibility.
The initial device profile conservatively targets H.264/AAC MP4 and HLS.
Web uses native HLS when available, otherwise pinned `hls.js` with compatible Media Source Extensions.
Autoplay refusal leaves an explicit Play action.

The player retains its session through ordinary rerenders, viewport changes, and fullscreen changes.
Close, navigation away, profile mutation, and unmount stop local media and submit the final known logical position.
Route departure unmounts the player before destination metadata reads finish.
The exit position remains fixed even if video detachment or a late response changes element state.
**Check session state** performs a read; an unchanged transport preserves position and pause state.
Create and transport retries retain their exact UUID and body in memory.
Reports never replay automatically after ambiguous failures.
Unknown personal outcomes stop media; explicit session checks do not invent provider success.
Page reload discards playback authority; lease expiry and durable cleanup handle abandoned sessions.

## Native players

Movie and Episode details expose explicit Play with the current source/library scope and live personal resume policy.
Catalog history restores presentation context, not playback sessions.
Process restart and a return after a background stop require explicit Play from details.
No native client exposes Maintenance, operator onboarding, provider credentials, Picture in Picture, or background playback.

| Family | Engine and presentation | Media limits |
| --- | --- | --- |
| Apple: iOS, iPadOS, macOS, tvOS | AVPlayer with AVKit surfaces and SwiftUI controls. iOS uses fullscreen presentation; macOS uses window fullscreen; tvOS occupies the screen. | H.264/AAC MP4, native HLS, plain WebVTT captions, and gateway burn-in choices. Custom WebVTT styles and cue placement remain absent. |
| Android mobile and TV | Media3 ExoPlayer with an embedded PlayerView and Compose controls. Immersive fullscreen hides system bars; TV exposes remote focus and media keys. | MP4, HLS, gateway WebVTT, and gateway burn-in choices. Actual decoder support requires device acceptance. |
| Linux | Embedded GStreamer `playbin` and `gtk4paintablesink` in a GTK window; no external-player handoff. | Installed MP4/H.264/AAC, HTTP, HLS, and subtitle plugins determine advertised capabilities. Absent required plugins produce an error. |

The engines fetch video, HLS, and subtitle bytes directly from gateway media URLs, outside the shared core.
The core owns session control and validates gateway descriptors; native clients never receive provider credentials or provider media URLs.
Native controls expose play/pause, seek, timeline labels, mute, and volume.
Track controls use gateway choices, explicit subtitles Off, and at most one explicit compatible-transcode fallback.
Playback settings expose session checks and exact-request preparation/transport retries.
Capabilities, media URLs, mutation bodies, and session track choices remain in memory.

Native controllers serialize reports by event sequence, not by position.
They apply the transport offset once and retain valid backward positions.
Apple and Android request a new gateway timeline for seeks before the current transport offset.
Linux also requests a new gateway timeline for pre-offset seeks, with the exact logical target in a serialized transport update.
Its seek slider spans zero through the logical duration.
The replacement applies the new offset once; exact retries retain the transport update's UUID and body.
Ambiguous personal reports stop or suspend local media and block later reports.
A session read never proves the provider's personal-state outcome or authorizes automatic report replay.
Preparation and transport retries retain their exact mutation UUID and body.

### Native stop and restoration

All three native families detach local media before remote cleanup completes and freeze the final logical position.
For known sessions, they submit Stop after prior control work settles.
Abandoned sessions still depend on gateway lease expiry and durable cleanup.
Source implements these lifecycle hooks:

| Family | Local stop triggers | Stop uncertainty |
| --- | --- | --- |
| Apple | Exit, true unmount, profile change, and any scene phase other than active. Fullscreen presentation preserves the session. | Explicit Stop retry reuses the frozen position, UUID, and body. Session checks preserve earlier personal-report uncertainty. |
| Android | Exit, Android Back outside fullscreen, lifecycle `ON_STOP`, composition disposal, and client changes. | The controller retains the Stop response and distinguishes local-only, pending, uncertain, confirmed, and recovery-expired outcomes. No automatic Stop retry occurs. |
| Linux | Close, Escape, navigation, profile change, replacement, disposal, and loss of active player-window focus after initial activation. | An ambiguous Stop triggers a state read, not a repeated write. An application hold permits bounded cleanup after window close. |

Apple stops even on an inactive scene transition; Linux treats window focus loss as background.
Apple also stops on background transitions; active scenes never resume or recreate a stopped owner.
The user returns to details and presses Play for a new session.
Apple fullscreen transitions retain the same player, session, and report sequence without control mutations.
On iOS, presentation-only disappearance skips Stop; true presenter unmount closes playback.
Actual scene deactivation still stops playback during fullscreen.
The tvOS Play/Pause command uses the same report-aware toggle as the onscreen control, with busy, blocked, closed, and session guards.
Apple restores entry-owned snapshots before scroll and focus.
Snapshots retain pagination, search context, results, and errors within bounds of 16 entries and 10,000 records.
Profile, source, client, and read-generation fences reject stale restoration.
Android stops on `ON_STOP`, not every transient focus loss.
Android fullscreen changes preserve the player, session, and event sequence; Back first exits fullscreen without Stop.
After Stop, explicit exact-request retry and read-only cleanup checks remain available in the player or same-profile catalog.
Android retains that recovery authority only in memory for up to 60 seconds, including after player exit.
The cleanup owner retains its own Client; confirmation or the deadline closes it and discards authority.
Expiry never implies cleanup confirmation.
New playback waits while recovery remains open.
Client disposal permits bounded cleanup to finish independently.
Android fences recovery ownership before profile mutation dispatch, even when the mutation receipt remains ambiguous.
Only current-owner recovery controls remain visible or block new playback; stale callbacks and actions cannot reclaim that authority.
Old controllers retain their private cleanup Client and deadline independently of visible recovery ownership.
Android catalog restoration retains bounded entry-owned results, continuation tokens, query/options/scope, scroll positions, and TV focus.
Refresh and profile/client changes invalidate those reads.
Scroll restoration waits at most 1.5 seconds before it clamps the saved position to available content.
These hooks do not imply runtime acceptance across operating-system versions or devices.
Apple initially prepares paused and requires the player's Play action; Android and Linux attempt playback after detail Play.

The platform guides own reproducible prerequisites and checks:
[Apple](../apps/apple/README.md#native-playback), [Android](../apps/android/README.md#playback), and [Linux](../apps/linux/README.md#native-playback).
Bindings, isolated controller tests, and toolchain builds do not establish native decode, subtitle synchronization, focus, or live-provider acceptance.
The [testing guide](testing.md#native-playback-checks) keeps these evidence boundaries separate.

## Jellyfin identity and personal state

The gateway creates a distinct managed Jellyfin device session for each logical playback session.
Quick Connect authorizes only the gateway's own code for the validated personal profile.
That current personal link can name the installation's indexing identity, but playback never substitutes that identity for an absent link.
The gateway verifies returned user and server identities.
Negotiation, media, and reports use that personal token.
The administrator key serves only connection checks, exact authorization, and narrowly scoped recovery.
Disabled Quick Connect returns `playback_authorization_unavailable`; no administrator-only report fallback exists.
The gateway never enables Quick Connect automatically or asks a client for a Jellyfin password.

Private token files remain separate from PostgreSQL session metadata.
Private files provide access controls, not encryption at rest.
Stop revokes local media before remote cleanup.
Cleanup targets only the exact gateway-owned device and play-session identities.
Transport replacement never fabricates an item stop or duplicate item start.
Durable cleanup survives gateway restarts.

Jellyfin remains authoritative for resume positions and watched status.
Personal-state mutations invalidate old watched snapshots through a separate freshness epoch.
Acquisition captures that epoch; publication requires an unchanged epoch and no pending provider effect.
Confirmed effects advance the epoch before another acquisition.
Playback events do not increment the personal binding revision.
An ambiguous effect retains unknown freshness until remote completion or controlled provider quiescence has sufficient evidence.
A timeout, fresh read, token revocation, or elapsed grace period alone never clears that uncertainty.
An exhausted reconciliation budget does not release the fence.
Watched values remain unknown rather than false or a percentage-derived completion claim.
Continue-watching remains a live provider read.
Web invalidates its local reads after confirmed playback reports.

DRM, live TV, multi-part playback, external URL media, downloads, casting, and authentication remain absent.
Safari, native engines, and real-provider behavior require separate acceptance evidence.

## Source contracts

- [Gateway orchestration](../crates/screenroom-playback-gateway/src/service.rs), [public records](../crates/screenroom-playback-gateway/src/records.rs), and [HTTP transport](../crates/screenroom-playback-gateway/src/transport.rs).
- [Durable authority](../crates/screenroom-backend/src/playback/mod.rs) and [watched freshness](../crates/screenroom-backend/src/watched.rs).
- [Jellyfin device protocol](../crates/screenroom-catalog-adapter/src/jellyfin/playback.rs), [negotiation](../crates/screenroom-catalog-adapter/src/jellyfin/playback/negotiation.rs), and [resource validation](../crates/screenroom-catalog-adapter/src/jellyfin/playback/resources.rs).
- [Shared client](../crates/screenroom-client-core/src/playback.rs), [Web controller](../apps/web/src/lib/core/playback.ts), and [player](../apps/web/src/lib/components/Player.svelte).
- [Apple controller and AVKit surfaces](../apps/apple/Sources/Home/NativePlayback.swift).
- [Apple entry snapshots and read fences](../apps/apple/Sources/Home/Catalog.swift) and [navigation, scroll, and focus restoration](../apps/apple/Sources/Home/HomeView.swift).
- [Android controller](../apps/android/ui/src/main/kotlin/org/screenroom/ui/NativePlayback.kt), [gateway adapter](../apps/android/ui/src/main/kotlin/org/screenroom/ui/PlaybackGateway.kt), and [controls/lifecycle](../apps/android/ui/src/main/kotlin/org/screenroom/ui/PlaybackScreen.kt).
- [Android presentation state](../apps/android/ui/src/main/kotlin/org/screenroom/ui/PlaybackPresentation.kt), [timeline](../apps/android/ui/src/main/kotlin/org/screenroom/ui/PlaybackTimeline.kt), [catalog restoration](../apps/android/ui/src/main/kotlin/org/screenroom/ui/HomeScreen.kt), and [entry-owned pages](../apps/android/ui/src/main/kotlin/org/screenroom/ui/EntryPages.kt).
- [Linux controller and GStreamer surface](../apps/linux/src/playback.rs) and [timeline/report state](../apps/linux/src/playback_state.rs).
