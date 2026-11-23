# Index

| Path | Info |
| --- | --- |
| [src/lib/core/](src/lib/core/) | Shared WASM client integration and UI state. |
| [src/lib/components/](src/lib/components/) | Svelte UI, profile controls, and indexing controls. |
| [vite.config.ts](vite.config.ts) | SPA fallback and local API configuration. |
| [scripts/prepare-core.mjs](scripts/prepare-core.mjs) | WASM preparation and cache inputs. |
| [tests/](tests/) | UI and state tests. |
| [README.md](README.md) | Application entry points and test prerequisites. |

# Contracts

- This application uses the SPA fallback; the static website does not.
- Use shared WASM clients for public and operator APIs; do not add direct provider HTTP.
- Derive `OperatorClient` from the same configured `Client` and use the shared gRPC-Web transport.
- Keep private targets and credentials server-side; a capability selector supplies no authentication.
- Preserve wide JSON integers without JavaScript Number coercion.
- Keep profile-bound requests, caches, and UI state isolated by client generation.
- Older maintenance APIs and browser operation-journal replay are not supported; do not retain compatibility UI.
- Web Jellyfin setup uses shared operator discovery/save RPCs; first-add supports no edit or rotation workflow.
- Retain mutation UUIDs and exact bodies only in memory for explicit same-session Retry; never replay on reload or page entry.
- Clear the setup key on completion, cancel, and unmount; URL/key changes invalidate discovery and selections.
- Refresh requires saved settings; no-policy Refresh must not create a configure-to-start chain.
- Do not persist API keys in browser storage, exports, logs, or URL parameters.
- Agent operational approvals do not require product acknowledgement checkboxes or manual request-UUID forms.

# Hints

- Use `changing-client-contracts` for shared client, proxy DTO, or model changes.
- Use `changing-providers` for setup and credential ownership changes.
- Use `changing-indexing` for policy and run semantics.
