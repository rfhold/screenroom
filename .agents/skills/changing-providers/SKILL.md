---
name: changing-providers
description: Use when changing Screenroom Jellyfin or provider transport, onboarding, API-key ownership or rotation, private connections, source revisions, delta capabilities, or profile generations. Excludes queue mechanics and client generation alone.
---

# Changing Providers

Supply provider inputs to installed `planning-changes`, `making-changes`, and local [reviewing-changes](../reviewing-changes/SKILL.md).
This skill owns no edit sequence, review process, fallback, commit, or deployment workflow.

## Canonical Guidance

- [Provider boundary](../../../crates/screenroom-catalog-adapter/AGENTS.md)
- [Backend operations](../../../docs/backend.md)
- [Managed Jellyfin connections and private credentials](../../../docs/jellyfin.md)
- [Indexing and Jellyfin delta contracts](../../../docs/indexing.md)
- [Shared client boundary](../../../crates/screenroom-client-core/AGENTS.md)

For queue mechanics or domain admission changes, also use [changing-indexing](../changing-indexing/SKILL.md).
For public DTOs, profile APIs, or setup UI contracts, also use [changing-client-contracts](../changing-client-contracts/SKILL.md).
Do not use this skill for platform-only presentation changes.

## Invariants

- Backend adapters own provider HTTP; browser and native clients never call Jellyfin directly.
- Keep request timeouts, response bounds, cancellation, retry classification, and sanitized errors explicit.
- Private keys, file locators, source revisions, and private profile bindings never enter public DTOs or telemetry.
- Source and profile identifiers select data; neither identifiers nor private networks authenticate users.
- Adapter locator digests exclude key contents and remote server state; managed connection revisions use a separate contract.
- Managed revisions bind connection identity, credential reference, authenticated server identity, and normalized URL; source admission also checks connection generation.
- Runtime resolution separately verifies ServerId; a revision proves neither credential freshness nor remote dataset identity.
- Check connection lifetime, source admission, and profile generations separately when credentials change.
- Keep personal watched state separate from shared catalog metadata.
- Delta polls need overlap, durable checkpoints/windows, capability fallback, and periodic complete reconciliation.
- LibraryChanged supplies optional future hints, not a required journal; Kodi integration remains optional future work.
- The inspected Jellyfin 10.10.7 filter defect lacks live reproduction; do not generalize it across all 10.10 versions.
- Managed backend connections use private key files, private database references, and dynamic user/worker resolution.
- First-add is the supported API; edit, credential rotation, and connection/server replacement have no supported API or UI workflow.
- Web setup uses shared operator discovery/save RPCs; native apps supply no onboarding UI.
- Browser setup retains keys only in memory; completion, cancel, and unmount clear them. Explicit Retry preserves the same in-memory UUID/body.
- Page entry and reload read server state; no browser journal or automatic mutation replay exists.
- Private filesystem storage provides no encryption at rest; source guards do not prove live container ownership or mount permissions.
- Runtime product mocks, TMDB catalog fixtures, and demo-user bootstrap are not supported; isolated test doubles remain test-only.

### Destination Security

- Allow HTTP and HTTPS destinations on LAN and loopback networks.
- Reject URL userinfo, query strings, and fragments.
- Reject redirects and environment-proxy use.
- Reject unspecified, multicast, broadcast, link-local, and cloud-metadata destinations, including mapped IPv4 equivalents.
- Validate every DNS answer before any connection.
- Connect only to validated addresses; never perform a second hostname lookup between validation and connection.
- Preserve the original Host header, TLS SNI, hostname verification, and configured base paths.

## Coupled Paths

Paths below start at the repository root.

| Change | Also inspect |
| --- | --- |
| Transport or connection identity | `crates/screenroom-catalog-adapter/src/jellyfin.rs`, `src/jellyfin/{revision,connection,secret,network}.rs` within that crate; backend managed resolution |
| Credential setup or change | `crates/screenroom-backend/src/connections.rs`, `crates/screenroom-backend/src/connections/secrets.rs`, `migrations/`, worker admission, user-service resolution |
| Profiles and personal state | `crates/screenroom-user/src/{profiles,watched,progress}.rs`, adapter user-state/watched contracts, shared client, all application profile consumers |
| Incremental traversal | Adapter `catalog_delta.rs` and `jellyfin/delta.rs`, `crates/screenroom-index-worker/src/full.rs`, backend indexing checkpoints |
| Web-first setup | `apps/web/src/lib/`, `proto/screenroom/operator/v1/`, operator server and shared-client DTOs; inspect actual routes before claims |

## Verification Inputs

- Use [focused check inputs](../reviewing-changes/references/checks.md) for adapter tests and disposable backend integration tests.
- Include malformed locators, bounded responses, capability fallback, source disablement, credential redaction, and stale profile responses.
- Inspect adapter `src/jellyfin/{tests.rs,revision/tests.rs,delta/tests.rs}` and user-service profile tests for relevant coverage.
- Inspect `crates/screenroom-backend/tests/connections.rs` and private-file tests in `crates/screenroom-backend/src/connections/secrets.rs`.
- Check file owner/mode, descriptor-relative no-follow access, no-overwrite publication, and uncertain-commit file retention.
- Inspect [network tests](../../../crates/screenroom-catalog-adapter/src/jellyfin/network/tests.rs) for mixed DNS answers, second-lookup prevention, address bounds, and DNS deadlines.
- Include URL rejection, redirect/proxy rejection, mapped IPv4, Host/TLS identity, and base-path preservation in affected transport cases.
- Local fault tests and source inspection do not establish live TLS or container credential-volume acceptance.
- Onboarding or rotation routes through private connections, revision semantics, profile generations, and public redaction—not client direct HTTP.

## Forbidden Or Gated Operations

- Do not put API keys in browser storage, command arguments, URLs, public fixtures, logs, or exports.
- Do not infer live provider access, imports, key-file writes, or credential rotation authority from a code-change request.
- Do not turn operational agent approval into product setup confirmation gates.
- Do not copy sibling authentication architecture or infrastructure policy into Screenroom.
