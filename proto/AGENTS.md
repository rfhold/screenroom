# Index

| Path | Info |
| --- | --- |
| [screenroom/catalog/v1/catalog.proto](screenroom/catalog/v1/catalog.proto) | Public browse, catalog, and search services. |
| [screenroom/user/v1/user.proto](screenroom/user/v1/user.proto) | Public user, profile, and personal-state services. |
| [screenroom/common/v1/common.proto](screenroom/common/v1/common.proto) | Shared public search types and methods. |
| [screenroom/user/internal/](screenroom/user/internal/) | Private user-internal package. |
| [../buf.yaml](../buf.yaml) | Buf module and lint inputs. |
| [../buf.gen.yaml](../buf.gen.yaml) | Generated Rust owner configuration. |

# Contracts

- Keep public services separate from private operator and internal packages.
- Keep provider keys, file locators, private connection references, and operator profile bindings outside public fields.
- Treat source and profile identifiers as selectors, not authorization.
- Preserve empty adapter and playback packages unless the task explicitly implements them.
- Wire changes also affect generated Rust, domain records, native bindings, WASM models, services, and application consumers.
- Do not add compatibility aliases for contracts that the approved hard cutover removes.

# Hints

- Use `changing-client-contracts` for field, RPC, namespace, or pagination changes.
- Read [generated ownership](../crates/screenroom-proto/AGENTS.md) before generation.
