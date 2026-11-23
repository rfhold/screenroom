<svelte:head>
  <title>Docs · Screenroom</title>
  <meta name="description" content="Screenroom’s local backend, Jellyfin integration, and requirements for production authentication." />
</svelte:head>

<div class="container">
  <header class="page-intro"><p class="eyebrow">Documentation</p><h1>The next chapter.</h1><p class="lead">Public documentation is not ready yet. Repository guides cover the persistent local backend and shared Rust core. No production service exists.</p></header>
  <div class="prose">
    <section aria-labelledby="backend-title">
      <h2 id="backend-title">Intended backend architecture.</h2>
      <p>Catalog and user services expose local gRPC and gRPC-Web reads over PostgreSQL. A durable worker indexes media from managed Jellyfin connections. Clients store preferences locally.</p>
      <p>The playback gateway manages Jellyfin playback sessions and serves media to clients.</p>
      <p>The intended business responsibilities are:</p>
      <ul>
        <li><code>screenroom-user</code>: user accounts and playback-provider associations where supported, with Jellyfin first.</li>
        <li><code>screenroom-catalog</code>: the catalog index, embeddings, and links between items in other catalogs.</li>
        <li><code>screenroom-playback-gateway</code>: a standalone executable for playback-provider integration.</li>
      </ul>
      <p>The intended deployment places these three services behind one public DNS hostname. Ingress routes requests by fully qualified gRPC service/method path or HTTP path. DNS resolves the hostname; it does not route request paths.</p>
      <p>The intended production contract requires each backend to validate authentication independently. Ingress authentication alone does not establish trust.</p>
      <p>The local Compose stack uses Envoy path routes on loopback, not a production deployment. It provides no authentication verifier. Shared telemetry supplies logs, trace context, and optional OTLP trace export.</p>
      <p>Catalog RPCs use <code>screenroom.catalog.v1</code>; UserService uses <code>screenroom.user.v1</code>. Shared search types use <code>screenroom.common.v1</code>. No legacy preference-service aliases exist.</p>
      <p>Exact vector search covers eligible source documents and reports lexical degradation explicitly. Private Garage storage supplies verified artwork through the same public origin. Local user UUIDs select state. They do not authenticate callers. Private operator and internal RPCs stay outside public edge routes.</p>
    </section>
    <div class="notice"><h2>For now, take a look around.</h2><p>The showcase offers actual client screenshots. About explains the project’s purpose, current state, and future direction.</p><div class="actions"><a class="button" href="/showcase/">Explore the showcase <span aria-hidden="true">↗</span></a><a class="text-link" href="/about/">About Screenroom</a></div></div>
  </div>
</div>
