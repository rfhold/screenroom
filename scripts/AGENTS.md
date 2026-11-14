# Index

| Path | Info |
| --- | --- |
| [build-client-core.sh](build-client-core.sh) | Native, UniFFI, WASM, Apple, and Android artifacts. |
| [generate-client-models.py](generate-client-models.py) | Ordinary Rust and TypeScript domain records. |
| [patch-swift-cancellation.py](patch-swift-cancellation.py) | Generated Swift cancellation and ownership corrections. |
| [check-client-boundary.mjs](check-client-boundary.mjs) | Public client boundary checks. |
| [backend-lifecycle.py](backend-lifecycle.py) | Persistent local instance lifecycle and descriptor identity. |
| [e2e.sh](e2e.sh) | E2E orchestration entry point. |
| [../docs/backend.md](../docs/backend.md) | Persistent instance operations and recovery. |

# Boundaries

Generation, validation, lifecycle, and device orchestration share this directory. A script path alone grants no execution authority.

# Contracts

- Inspect modes, environment inputs, outputs, and target selection before invocation.
- Preserve caller-owned teardown and persistent instance identity.
- Keep generator output ownership explicit; do not overwrite unrelated dirty output.
- Keep API keys and private file locators out of command arguments, logs, and generated public assets.
- Do not preserve product mock or TMDB import entry points as compatibility requirements for the approved cutover.
- Test fixtures and isolated test doubles remain distinct from runtime product data.

# Hints

- Use [review checks](../.agents/skills/reviewing-changes/references/checks.md) to distinguish local checks from runtime operations.
- Generation changes require dependent cache and cancellation checks, not just a successful generator exit.
