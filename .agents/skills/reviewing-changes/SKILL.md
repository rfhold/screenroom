---
name: reviewing-changes
description: Use for Screenroom change reviews that need repository-specific queue, provider, client, proxy, migration, publication, search, or platform risks and validation inputs.
---

# Screenroom Review Inputs

Supply these inputs to installed `making-changes`. This skill does not own review sequencing, fallback, commits, or deployment.

## Canonical Guidance

- [Root instructions](../../../AGENTS.md) route to scoped boundaries.
- [Documentation index](../../../docs/README.md) routes to behavior and operations guidance.
- [Risk inputs](references/risks.md) cover coupled source contracts and security boundaries.
- [Check inputs](references/checks.md) cover commands, prerequisites, side effects, and evidence.

Load the focused references for the affected surface. Inspect current source before implementation claims.
Treat documented target behavior separately from completed source behavior.

## Area Inputs

| Change surface | Additional skill |
| --- | --- |
| Queue, retry, admission, worker, policy, publication | [changing-indexing](../changing-indexing/SKILL.md) |
| Provider transport, credentials, revisions, profiles | [changing-providers](../changing-providers/SKILL.md) |
| Wire fields, client APIs, bindings, proxy DTOs, consumer parity | [changing-client-contracts](../changing-client-contracts/SKILL.md) |

## Evidence Inputs

- Report affected paths, checks, results, and concrete blockers in the session.
- Distinguish passed, failed, and unrun checks.
- Inspect untracked files as well as tracked diffs.
- A build or zero-test run does not prove behavior or coverage.
- Keep operational approvals separate from application UX.
- Local checks grant no service lifecycle, provider import, credential rotation, or external mutation authority.
