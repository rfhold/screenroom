# Index

| Path | Info |
| --- | --- |
| [README.md](README.md) | Documentation routing. |
| [components.md](components.md) | Product scope and component boundaries. |
| [backend.md](backend.md) | Lifecycle, migrations, persistent identity, and operations. |
| [indexing.md](indexing.md) | Canonical indexing architecture and policy contracts. |
| [index-maintenance.md](index-maintenance.md) | Artwork repair, publication, operator security, and recovery context. |
| [client-api.md](client-api.md) | Public client, separate operator capability, and bindings. |
| [jellyfin.md](jellyfin.md) | Managed setup, private credentials, destination security, and first-add limits. |
| [testing.md](testing.md) | Isolated tests, distinct attached acceptance modes, and platform limits. |

# Contracts

- Distinguish approved target behavior from verified implementation facts.
- Source files prove implementation; component scope alone does not.
- Keep lifecycle procedures in `backend.md` and indexing architecture in `indexing.md`.
- Preserve durable publication, security, and recovery facts when superseded interfaces disappear.
- Do not promote obsolete browser journals or older maintenance APIs into compatibility requirements.
- Keep credentials, private connection references, and private locators out of examples.
- Keep one-off validation results, session plans, and signoff dates outside canonical prose.
- Add links to future documents only after their paths exist.

# Hints

- Use installed `agentic-documentation` for documentation and instruction changes.
- Check untracked Markdown files directly; tracked diffs do not cover their links or whitespace.
