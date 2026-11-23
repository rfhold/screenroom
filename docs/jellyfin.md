# Jellyfin connections

The backend supports managed Jellyfin connections with private API-key files and dynamic database discovery.
Web Maintenance uses shared client RPCs for URL and administrator API-key setup.
Native clients use ordinary shared-core reads, not native onboarding or direct Jellyfin HTTP.
The [backend guide](backend.md) owns runtime lifecycle; the [indexing guide](indexing.md) owns subsequent acquisition and repair.

## Web setup

Pantheon supplies the backend origin and local user at runtime. Local development uses the configured backend origin and enabled local user.
`VITE_SCREENROOM_API_ENDPOINT` and `VITE_SCREENROOM_USER_ID` select those values; `SCREENROOM_API_ENDPOINT` and `SCREENROOM_USER_ID` supply aliases.
Explicit `VITE_` values take precedence for local builds.
An absent or malformed local user UUID disables setup submission; the backend also requires that the user remains enabled.
The UUID selects the current user; it does not authenticate an administrator.

Open Maintenance, or enter `/?screen=maintenance`, then:

1. Choose **Add server**.
2. Enter the backend-reachable Jellyfin base URL and administrator API key.
3. Choose **Connect** for authenticated discovery.
4. Select an enabled Jellyfin profile.
5. Select 1–32 distinct supported libraries, with at most one collections library.
6. Choose **Save**, or explicitly choose **Save and refresh**.

Unsupported library choices remain visible but unavailable for selection.
URL or key changes invalidate discovery, the profile selection, and library selections.
Browser memory holds the key only for the current setup session; success, Cancel setup, and unmount clear it.
No key enters browser storage, exported records, URLs, or logs.
The backend keeps private files, not encrypted-at-rest credentials.

An ambiguous save offers **Retry save** with the exact same in-memory UUID and body.
The app never retries save automatically. **Reload servers** checks current state without another save.
Cancel or reload discards the in-memory request; neither proves that a submitted server mutation stopped.
Older valid or corrupt browser journals receive no reads, replay, migration, export, or recovery prompts.

Save selects the new source and saved policy. Save and refresh also opens the admitted run.
Subsequent Refresh uses that saved enabled policy; Advanced settings expose policy, history, and queue details.
Without a saved policy, Refresh stays disabled; it never creates a configure-to-start chain.
Native apps supply no setup UI. These controls add no authentication, edit, rotation, or replacement workflow.

## Backend setup contract

Discovery requests authenticated `System/Info`, libraries, and profiles. Public version information alone does not prove access.
Discovery persists no connection, key, policy, or queue job.
Operator logs report failed or cancelled probe steps with elapsed time. Failures include a safe category and, where available, a numeric HTTP status.
Use the `Jellyfin connection probe` event to distinguish server identity, library, profile, and response validation failures.
The event excludes provider URLs, credentials, response bodies, server names, and identifiers.
Successful steps appear at debug level. The [shared telemetry configuration](backend.md#shared-telemetry) controls RPC trace export.
Save probes again with the exact submitted key and verifies the selected server identity, profile, and libraries.
A stale discovery response cannot substitute for that check.

Save accepts 1–32 distinct supported libraries, with at most one collections library.
Movie, television, and collections roots are supported; other kinds remain visible as unsupported discovery choices.
The selected profile supplies the installation indexing binding and the current local user's personal Jellyfin link.
Other users' links remain unchanged. The local UUID selects a user; it does not authenticate one.

Save creates the private connection, source/libraries, profile binding, and canonical policy.
Defaults enable metadata and artwork, disable embeddings, and leave automatic runs off.
Save alone enqueues no jobs. Explicit Save and Refresh provisions the connection, policy, run, and jobs in one database transaction.
The response carries the safe connection summary, policy UUID, exact `i64` policy version, and nullable run UUID.
User and worker services discover managed connections dynamically; they need no restart, manual provider JSON, or enrollment command.
Runtime acquisition verifies the actual remote ServerId; a locator digest alone does not prove dataset identity.

## Connection API

These RPCs use the [trusted operator boundary](index-maintenance.md#browser-boundary).
They add no administrator authentication and remain separate from the catalog service.

| RPC | Payload or result |
| --- | --- |
| `DiscoverConnection` | `{base_url, api_key}`; safe server identity, libraries, and enabled profiles. |
| `SaveConnection` | `{request_id, base_url, api_key, server_id, profile_id, library_ids, local_user_id, refresh}`; saved connection/policy/run result. |
| `ListConnections` | Paginated safe connection summaries with UUID cursor; limit 1–200, default 100. |

`refresh` defaults to false. Save uses a stable request UUID per intended operation.
An identical in-session request retains that UUID; reuse with different input conflicts.
Save receipts support identical replay for 24 hours and compare the key against the private credential file, not a durable key digest.
This idempotency contract requires no browser journal, reload replay, UUID export, or old-origin migration.
Safe summaries expose connection/source UUIDs, source key, display name, server version, and state.
They exclude private connection URLs, credential references, locator revisions, and key material.

## Credential ownership

API keys exist transiently in browser/backend memory and in private backend files.
Keys never enter database JSON, browser localStorage, public DTOs, logs, URLs, or command arguments.
The database stores a private credential reference and connection metadata, not the key itself.
This design does not provide encryption at rest.

`SCREENROOM_JELLYFIN_CREDENTIAL_DIRECTORY` selects the private backend directory.
Compose mounts `/run/screenroom/credentials` from the persistent `backend-jellyfin-credentials` volume.
The directory requires mode `0700`; generated UUID-named files require `0600` and the service's effective UID.
Compose services use UID `10001`; operator mounts the volume read/write.
User, worker, and playback gateway mount it read-only; catalog receives no mount.
Descriptor-relative no-follow access rejects symlinks, hard-linked files, wrong owners/modes, special files, and oversized keys.
Files publish with no overwrite and filesystem synchronization.
An uncertain database commit never deletes a file that committed state could reference.

The operator uses the separate `operator_runtime` login with `screenroom_operator` grants.
User, worker, and playback gateway receive restricted private connection reads; catalog receives no connection or credential access.
Migration credentials remain a separate process capability, not a claim of process-level privilege isolation.
The lifecycle helper initializes a new empty credential volume's ownership; it refuses arbitrary repair of populated or mismatched stores.
Source configuration and guards do not prove actual ownership or mount permissions in a particular live installation.
Preserve credential volumes with their database and runtime identity. This guide authorizes no credential deletion, rotation, or volume reset.

## Personal playback authorization

The [playback contract](playback.md#jellyfin-identity-and-personal-state) uses a backend-owned Jellyfin device session for each logical playback.
Playback requires the current validated personal profile link for the selected local user and source.
That link can name the installation's indexing profile, but an absent link never falls back to the indexing identity.
The trusted-local UUID selects the caller's state; it does not authenticate that caller.

Playback authorization requires an enabled Jellyfin Quick Connect feature.
The gateway initiates its own code and authorizes only that code for the current personal profile.
It verifies the returned user and server identities before media access.
Negotiation, media, and personal playback reports use the private device token, not the managed administrator key.
The administrator key serves connection/profile checks, exact code authorization, and exact-device cleanup or lost-token recovery.
Web receives neither the device token nor the temporary Quick Connect secret.
This flow adds no password form or external Jellyfin login step.
Disabled Quick Connect returns `playback_authorization_unavailable`; the gateway never enables the feature automatically.
No administrator-only personal-report fallback exists.

The [backend runtime guide](backend.md#playback-gateway-runtime) owns the separate private playback store, restricted role, readiness, and cleanup.
Stop, expiry, replacement, and restart retain exact cleanup intent; cleanup never deletes devices through profile-wide filters.
Private files provide access controls, not encryption at rest.
Source implementation does not establish live profile attribution, burn-in, audio selection, TLS, or Safari compatibility.
Real-provider acceptance requires separate approval for the exact installation, server, profile, titles, lifecycle, and provider writes.

## Destination and transport limits

HTTP and HTTPS allow LAN and loopback destinations. Plain HTTP provides no transport encryption; use a trusted network or HTTPS.
Inside a container, `localhost` identifies that container, not the host or another service.
Use a reachable provider address from the backend's network.
These explanations require no acknowledgement gate.

The adapter rejects URL userinfo, query strings, fragments, redirects, and environment proxies.
It rejects unspecified, multicast, broadcast, link-local, cloud-metadata, and unsafe transition addresses, including mapped IPv4 forms.
The resolver bounds DNS results and its deadline, validates every answer before connection, and pins validated addresses.
It performs no second hostname lookup between validation and connection.
The transport preserves the original Host, TLS SNI, certificate hostname checks, and base paths.
Managed discovery uses a three-second request timeout and an 8 MiB response bound.
The operator's request limit remains 65,536 bytes; oversized selections and reflected key material fail safely.

## Errors and unsupported actions

| Safe code or condition | Meaning |
| --- | --- |
| `credentials_rejected` | Jellyfin rejects the supplied key. |
| `invalid_connection`, `invalid_credentials` | The URL or key fails local validation. |
| `invalid_server_identity`, `server_identity_mismatch` | Authenticated identity is invalid or differs from the selected server. |
| `profile_unavailable`, `library_unavailable`, `unsupported_library` | Fresh discovery rejects a selection. |
| `selection_capacity`, `unsupported_library_scope` | The selection exceeds supported bounds. |
| `server_already_connected` | The server ID or normalized URL already has a managed connection. |
| `credential_store_unavailable`, `connection_unavailable`, `discovery_unavailable` | A private store, database, or provider dependency lacks availability. |
| `request_conflict`, `request_expired` | A save receipt cannot accept the submitted request. |

The API sanitizes upstream and database errors; raw responses are not troubleshooting output.
First-add is the supported setup operation. Edit, key rotation, connection replacement, and server replacement have no supported workflow.
Do not work around those limits with manual database changes or private JSON configuration.
Personal watched state remains [separate](backend.md#watched-snapshots) from catalog incremental checkpoints.
Timestamp polls and reconciliation have the [documented capability and deletion limits](indexing.md#jellyfin-incremental-acquisition).

## Source contracts

- [Managed connections](../crates/screenroom-backend/src/connections.rs) and [private files](../crates/screenroom-backend/src/connections/secrets.rs).
- [Authenticated adapter probe](../crates/screenroom-catalog-adapter/src/jellyfin/connection.rs) and [network guards](../crates/screenroom-catalog-adapter/src/jellyfin/network.rs).
- [Operator routes](../crates/screenroom-index-operator/src/lib.rs) and [Compose ownership](../compose.yaml).
