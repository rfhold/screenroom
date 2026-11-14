# Index

| Path | Info |
| --- | --- |
| [src/lib.rs](src/lib.rs) | Ordinary public `Client`, configuration, transport, and cancellation. |
| [src/models.rs](src/models.rs) | Generated ordinary domain records. |
| [src/wasm.rs](src/wasm.rs) | WASM ordinary client exports. |
| [src/wasm-models.ts](src/wasm-models.ts) | Generated ordinary TypeScript models. |
| [src/operator.rs](src/operator.rs) | Separate HTTP `OperatorClient`. |
| [src/operator_models.rs](src/operator_models.rs) | Operator domain records. |
| [src/operator_wasm.rs](src/operator_wasm.rs) | WASM operator exports. |
| [src/operator-models.ts](src/operator-models.ts) | Operator TypeScript model contract. |
| [../../docs/client-api.md](../../docs/client-api.md) | Shared client and binding contract. |

# Dependencies

Ordinary clients use public protobuf services. `OperatorClient` uses a separate explicit HTTP capability, not ordinary RPC configuration.

# Dependents

[Apple](../../apps/apple/), [Android](../../apps/android/), [Linux](../../apps/linux/), and [Web](../../apps/web/) consume this crate.

# Contracts

- Preserve native/WASM parity, explicit scope, immutable client configuration, bounded caches, and cancellation ownership.
- Keep prost, SQLx, provider credentials, and private backend records outside public domain models.
- Keep ordinary `Client` authority separate from `OperatorClient` authority.
- Preserve wide integer precision across Rust, UniFFI, WASM, TypeScript, JSON, and proxy transport.
- On profile changes, reject stale requests and invalidate profile-bound caches.
- Browser operation-journal replay and older maintenance APIs are not supported; do not add compatibility aliases.
- Regenerate ordinary models through [the model generator](../../scripts/generate-client-models.py), not isolated output edits.

# Hints

- Use `changing-client-contracts` for APIs, records, bindings, or transport changes.
- Inspect [the binding pipeline](../../scripts/build-client-core.sh) for UniFFI and WASM effects.
