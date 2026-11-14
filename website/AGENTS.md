# Index

| Path | Info |
| --- | --- |
| [src/routes/+layout.ts](src/routes/+layout.ts) | Prerender, no client JavaScript, and trailing-slash contract. |
| [vite.config.ts](vite.config.ts) | Static adapter without SPA fallback. |
| [static/404.html](static/404.html) | Explicit Cloudflare Pages 404 behavior. |
| [README.md](README.md) | Website development and deployment context. |

# Boundaries

This subtree owns the static public website, not the [Web SPA](../apps/web/AGENTS.md).

# Contracts

- Preserve prerendered routes and disabled client JavaScript.
- Preserve the explicit `build/404.html`; Cloudflare Pages otherwise supplies an implicit SPA fallback.
- Keep product API access, provider credentials, and operator capabilities outside this website.
- Keep public product claims consistent with verified source behavior.
- A successful build grants no Pages publication or deployment authority.

# Hints

- Select static-output checks from [review inputs](../.agents/skills/reviewing-changes/references/checks.md).
