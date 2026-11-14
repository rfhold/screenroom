# Index

| Path | Info |
| --- | --- |
| [../../proto/AGENTS.md](../../proto/AGENTS.md) | Source ownership and public/private package boundaries. |
| [../../buf.gen.yaml](../../buf.gen.yaml) | Rust generation configuration. |
| [src/generated/](src/generated/) | Buf-generated prost and tonic output. |
| [src/lib.rs](src/lib.rs) | Handwritten module exports and local-user metadata key. |

# Contracts

- Edit wire definitions in `proto/`, not generated Rust.
- Regenerate with the root Buf configuration when wire definitions change.
- Keep handwritten module exports aligned with generated packages.
- Preserve public/private namespace separation and comments-only output for empty packages.
- The local-user metadata key selects a trusted-local user; it supplies no authentication.

# Hints

- Use `changing-client-contracts` for public wire changes.
- Inspect generation drift and reproducibility through [review checks](../../.agents/skills/reviewing-changes/references/checks.md).
