---
name: changing-indexing
description: Use when changing Screenroom queue retry or redrive, indexing policy or admission, worker claims, traversal checkpoints, budgets, progress, or artwork publication. Excludes provider transport and client binding changes alone.
---

# Changing Indexing

Supply domain inputs to installed `planning-changes`, `making-changes`, and local [reviewing-changes](../reviewing-changes/SKILL.md).
This skill owns no edit sequence, review process, fallback, commit, or deployment workflow.

## Canonical Guidance

- [Indexing architecture](../../../docs/indexing.md)
- [Backend lifecycle and migrations](../../../docs/backend.md)
- [Artwork publication and repair context](../../../docs/index-maintenance.md)
- [Crate boundaries](../../../crates/AGENTS.md)

For provider HTTP, credentials, or capability discovery, use [changing-providers](../changing-providers/SKILL.md).
For wire models or exported APIs, also use [changing-client-contracts](../changing-client-contracts/SKILL.md).
Inspect current code when canonical prose still describes a superseded interface.

## Invariants

- One provider-neutral PostgreSQL queue serves indexing paths; domain repositories own eligibility and catalog effects.
- Preserve transactional enqueue, lease claims, fences, at-least-once idempotent effects, and durable attempt history.
- Keep run-scoped dependencies intact; content-byte dedupe cannot orphan another run.
- Central retry uses classified errors, bounded backoff, jitter, and Retry-After.
- Redrive preserves identity and history; it differs from paused-run resume and duplicate start-request handling.
- Preserve immutable policy/source snapshots; bind model identity and input hashes only when embeddings require them.
- Persist progress, traversal windows, checkpoints, lifetime budgets, and admission deadlines.
- Partial traversal never proves absence; enumeration completion alone does not prove successful settlement.
- Publication requires verified complete artwork variants and preserves the prior complete generation.
- Report current failure separately from historical resolved attempts.
- Keep diagnostics claimability distinct from raw queue state; use bounded metric labels.

## Coupled Paths

Paths below start at the repository root.

| Change | Also inspect |
| --- | --- |
| Retry, claims, redrive, dedupe | `crates/screenroom-queue/src/lib.rs`, `crates/screenroom-backend/src/{ingest,indexing,queue_admin}.rs`, `migrations/` |
| Worker execution or progress | `crates/screenroom-index-worker/src/{full,live,pipeline,repair}.rs`, backend admission and ingest |
| Publication or selection strategy | `crates/screenroom-backend/src/{repair,media,storage}.rs`, worker full/repair paths |
| Status, queue metrics, operator controls | `crates/screenroom-index-operator/src/lib.rs`, backend queue administration, `crates/screenroom-observability/src/{metrics,transport}.rs` |

## Verification Inputs

- Use [focused check inputs](../reviewing-changes/references/checks.md) with queue, backend, and worker tests.
- Inspect `crates/screenroom-backend/tests/{indexing,queue_cutover,full_traversal,repair}.rs` and `crates/screenroom-index-worker/tests/durable.rs`.
- Include stale-lease commits, retry exhaustion, duplicate redrive, budget preservation, blocked claims, and publication dependencies as affected cases.
- A retry-policy change routes through queue, domain admission, worker effects, and focused tests—not provider business logic.

## Forbidden Or Gated Operations

- Do not restore legacy queue views, older maintenance APIs, runtime product mocks, or browser operation-journal replay.
- Do not modify applied migrations or discard receipt provenance.
- Live worker activation, imports, migrations, and persistent-state changes require separate target-specific authority.
- Agent approval gates do not require application acknowledgement controls.
