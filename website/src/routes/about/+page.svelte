<svelte:head>
  <title>About · Screenroom</title>
  <meta name="description" content="Read about Screenroom’s native interfaces, shared Rust core, persistent local backend, and current development limits." />
</svelte:head>

<div class="container">
  <header class="page-intro"><p class="eyebrow">About Screenroom</p><h1>A considered home<br />for film & television.</h1><p class="lead">Screenroom explores a quieter way to find your next watch, with interfaces that feel at home on each screen.</p></header>
  <div class="prose">
    <section aria-labelledby="native-title"><h2 id="native-title">Native where it matters.</h2><p>A television remote is not a mouse. A phone is not a desktop. Each deserves its own layout, controls, and sense of space.</p><p>The client interfaces use SwiftUI on Apple platforms, Compose on Android, and GTK4 with libadwaita on Linux. The browser has a separate Svelte client.</p></section>
    <section aria-labelledby="direction-title"><h2 id="direction-title">A shared foundation.</h2><p>The clients use a shared Rust core to access a persistent local backend with protobuf contracts. Native clients use gRPC; the web client uses WebAssembly and gRPC-Web.</p><p>PostgreSQL stores the shared catalog, local users, durable ingest jobs, and search vectors. Typed offline adapters supply provider inputs; private object storage supplies artwork. Clients keep preferences locally. The playback gateway still serves empty routes.</p><p>The <a href="/docs/#backend-title">intended backend architecture</a> requires independent service authentication. Local UUID selectors and private RPC networks do not implement authentication or authorize production exposure.</p></section>
    <section aria-labelledby="today-title"><h2 id="today-title">Where we are today.</h2><p>Screenroom is an independent project in early development. Current clients offer Home, libraries, Search, and title details through the persistent backend.</p><p>There are no production services, authentication, live provider connections, playback, or releases yet.</p><p>This static website preserves historical fixture-backed screenshots; it is not the web client and does not connect to the API.</p><div class="actions"><a class="button" href="/showcase/">See the client previews <span aria-hidden="true">↗</span></a><a class="text-link" href="/docs/">Documentation status</a></div></section>
  </div>
</div>
