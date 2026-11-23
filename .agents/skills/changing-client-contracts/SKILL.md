---
name: changing-client-contracts
description: Use when changing Screenroom protobuf fields or RPCs, public or operator DTOs, shared Client APIs, UniFFI or WASM bindings, TypeScript models, cancellation, or cross-platform consumer parity. Excludes provider internals and queue mechanics alone.
---

# Changing Client Contracts

Supply contract inputs to installed `planning-changes`, `making-changes`, and local [reviewing-changes](../reviewing-changes/SKILL.md).
This skill owns no edit sequence, review process, fallback, commit, or deployment workflow.

## Canonical Guidance

- [Wire contract ownership](../../../proto/AGENTS.md)
- [Generated Rust ownership](../../../crates/screenroom-proto/AGENTS.md)
- [Shared client contracts](../../../docs/client-api.md)
- [Shared core boundary](../../../crates/screenroom-client-core/AGENTS.md)
- [Application boundaries](../../../apps/AGENTS.md)

For private provider connections or credentials, use [changing-providers](../changing-providers/SKILL.md).
For policy, retry, or run semantics behind an API, also use [changing-indexing](../changing-indexing/SKILL.md).
Platform-only styling does not require this skill unless it changes contract consumption.

## Invariants

- `Client.operator()` derives a distinct typed capability over the same configured RPC endpoint.
- Keep private RPCs, SQLx, provider keys, private locators, and operator profile bindings outside public models.
- Buf owns generated wire Rust; the model generator owns ordinary Rust and TypeScript domain records.
- Preserve native/WASM API parity, wide integers, nullable fields, errors, explicit scope, and immutable client configuration.
- Preserve cancellation ownership, bounded caches, same-origin artwork fetches, and profile-generation guards.
- Profile changes cannot expose previous-profile snapshots or accept stale responses.
- Operator RPCs retain payload bounds, lossless integers, and the trusted private network boundary.
- Neither operator attachment identifiers nor local user/profile selectors supply authentication.
- Older maintenance APIs and browser operation-journal replay are not supported; do not add compatibility surfaces.

## Coupled Paths

Paths below start at the repository root.

| Change | Also inspect |
| --- | --- |
| Public wire field or RPC | `proto/screenroom/{catalog,common,user}/v1/`, `buf.gen.yaml`, `crates/screenroom-proto/src/{generated,lib.rs}`, service implementations |
| Ordinary domain record | `scripts/generate-client-models.py`, client core `src/{models.rs,wasm.rs,wasm-models.ts}`, Rust-to-wire mappings |
| Operator DTO or method | `proto/screenroom/operator/v1/`, `crates/screenroom-index-operator/src/rpc.rs`, client core `src/{operator.rs,operator_models.rs,operator_wasm.rs,operator-models.ts}` |
| Native export or cancellation | `scripts/build-client-core.sh`, `scripts/patch-swift-cancellation.py`, core UniFFI config, Apple preparation, Android JNI preparation |
| Consumer parity | `apps/apple/Sources/`, `apps/android/ui/`, `apps/linux/src/`, `apps/web/src/lib/`, platform tests and generated-artifact cache keys |

## Verification Inputs

- Use [generation and platform check inputs](../reviewing-changes/references/checks.md).
- Check Buf generation drift, ordinary model `--check`, boundary tests, Rust/WASM parity, and platform consumer compilation as applicable.
- Include nullability, wide integer round trips, stale-profile cancellation, pagination scope, and public redaction in affected cases.
- A field change routes through proto, generated Rust, UniFFI, WASM, TypeScript, and all consumers—not private backend record exposure.
- Root Rust checks do not validate the independent Linux workspace or platform SDK builds.

## Forbidden Or Gated Operations

- Do not hand-edit generated bindings as the durable source or leak private backend fields into public models.
- Do not bypass the shared core with provider HTTP or ad hoc browser RPC clients.
- Generation commands grant no service lifecycle, device automation, provider access, or publication authority.
- Do not turn operational agent approval into application acknowledgement or request-UUID forms.
