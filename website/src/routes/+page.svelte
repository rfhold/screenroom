<svelte:head>
  <title>Screenroom · A place for what’s next</title>
  <meta name="description" content="Explore Screenroom client previews across mobile, desktop, TV, and the web, with a shared Rust core and persistent local backend." />
</svelte:head>

<div class="container">
  <section class="hero" aria-labelledby="hero-title">
    <div class="hero-copy">
      <p class="eyebrow">Film, television & a little less noise</p>
      <h1 id="hero-title">A place for<br />what’s next.</h1>
      <p class="lead">Screenroom is a film and television project with a simple aim: a thoughtful home for your next watch, on every screen.</p>
      <div class="actions">
        <a class="button" href="/showcase/">Explore the showcase <span aria-hidden="true">↗</span></a>
        <a class="text-link" href="/about/">Meet the project</a>
      </div>
    </div>
    <div class="hero-aside" aria-hidden="true"><span>SCREENROOM</span><div class="orbit"></div><span>IN DEVELOPMENT / 01</span></div>
  </section>

  <figure class="client-preview">
    <div class="preview-bar"><span><span class="status-dot"></span> A look inside</span><span>Web client / Home</span></div>
    <a href="/showcase/web/" aria-label="Explore the Web client screenshots">
      <div class="preview-window"><img src="/showcase/web/desktop.png" alt="Screenroom Web Home preview with illustrated film and series posters, title rows, and a compact sidebar." width="1440" height="2084" fetchpriority="high" /></div>
    </a>
    <figcaption><span>Historical client capture · Fixture-backed mock catalog</span><a href="/showcase/web/">See the full view <span aria-hidden="true">↗</span></a></figcaption>
  </figure>

  <section class="section" aria-labelledby="screens-title">
    <p class="eyebrow">Different screens. One intention.</p>
    <div class="section-heading"><h2 id="screens-title">At home on<br />your screen.</h2><p>A phone, a desktop, and a television ask for different things. Screenroom explores native interfaces that respect each platform, with a separate web client for the browser.</p></div>
    <div class="platforms">
      <a href="/showcase/ios/"><span class="platform-number">01 / MOBILE</span><h3>Within reach.</h3><p>iOS, iPadOS & Android</p><span class="platform-link">Explore mobile <span aria-hidden="true">↗</span></span></a>
      <a href="/showcase/macos/"><span class="platform-number">02 / DESKTOP</span><h3>A little more room.</h3><p>macOS, Linux & Web</p><span class="platform-link">Explore desktop <span aria-hidden="true">↗</span></span></a>
      <a href="/showcase/tvos/"><span class="platform-number">03 / TELEVISION</span><h3>The big screen.</h3><p>tvOS & Android TV</p><span class="platform-link">Explore television <span aria-hidden="true">↗</span></span></a>
    </div>
  </section>

  <section class="section approach" aria-labelledby="approach-title">
    <div><p class="eyebrow">The approach</p><h2 id="approach-title">Let the stories<br />take the spotlight.</h2></div>
    <div><p class="lead">Clear layouts. Familiar controls. Space to explore.</p><p class="muted">The current interfaces put the catalog first: Home, a Movies library, Search, and title details. The showcase follows these same flows across every client.</p><a class="text-link" href="/showcase/">Compare the client previews <span aria-hidden="true">↗</span></a></div>
  </section>

  <section class="development motif" aria-labelledby="development-title">
    <p class="eyebrow">An early look, not a launch</p>
    <h2 id="development-title">The first scenes.</h2>
    <p>Screenroom is in early development. Current clients use a shared Rust core and a persistent local backend with offline provider adapters. PostgreSQL stores catalog data and search vectors; private object storage supplies artwork. Authentication, live provider connections, production services, and playback are not available.</p>
    <p>There are no releases to download yet. For now, explore the interfaces and the direction behind them.</p>
    <a class="text-link" href="/about/">More about Screenroom <span aria-hidden="true">↗</span></a>
  </section>
</div>

<style>
  .hero { display: grid; grid-template-columns: 1fr 15rem; gap: var(--space-5); padding-block: var(--space-6); }
  .hero-copy { max-width: 49rem; }
  h1 { margin-bottom: var(--space-4); }
  .hero-aside { align-self: stretch; display: flex; flex-direction: column; justify-content: space-between; align-items: center; padding-block: var(--space-2); color: var(--text-secondary); font: 0.65rem var(--font-label); letter-spacing: 0.16em; }
  .orbit { width: 12rem; aspect-ratio: 1; border: 1px solid var(--border); border-radius: 50%; display: grid; place-items: center; background: radial-gradient(ellipse at bottom, var(--surface-raised), transparent 70%); }
  .orbit::before { content: ''; width: 7rem; height: 4.5rem; border: 1px solid var(--accent); border-radius: 50% 50% 0 0; transform: rotate(-25deg); opacity: 0.7; }
  .client-preview { margin: 0 0 var(--space-6); border: 1px solid var(--border); border-radius: var(--radius); background: var(--surface-inset); overflow: hidden; }
  .preview-bar, figcaption { display: flex; justify-content: space-between; flex-wrap: wrap; gap: var(--space-1) var(--space-3); padding: var(--space-2) var(--space-3); font-size: var(--text-small); color: var(--text-secondary); }
  .preview-bar { border-bottom: 1px solid var(--border); font: 0.7rem/1.6 var(--font-label); text-transform: uppercase; letter-spacing: 0.08em; }
  .status-dot { display: inline-block; width: 0.4rem; height: 0.4rem; margin-right: var(--space-1); background: var(--accent); border-radius: 50%; }
  .preview-window { aspect-ratio: 16 / 9; overflow: hidden; }
  .preview-window img { display: block; width: 100%; height: auto; }
  figcaption { border-top: 1px solid var(--border); }
  .platforms { display: grid; grid-template-columns: repeat(3, 1fr); gap: var(--space-4); }
  .platforms > a { border-top: 1px solid var(--border); padding-block: var(--space-3); color: var(--text-primary); text-decoration: none; }
  .platforms > a:hover { border-color: var(--accent); }
  .platform-number { font: 0.7rem var(--font-label); letter-spacing: 0.08em; color: var(--text-secondary); }
  .platforms h3 { font-size: 1.5rem; margin: var(--space-4) 0 var(--space-1); }
  .platforms p { color: var(--text-secondary); font-size: var(--text-small); }
  .platform-link { color: var(--accent); font-size: var(--text-small); }
  .approach { display: grid; grid-template-columns: 1fr 1fr; gap: var(--space-5); }
  .development { padding: clamp(1.5rem, 5vw, 4rem); margin-bottom: var(--space-6); border: 1px solid var(--border); border-radius: var(--radius); }
  .development p:not(.eyebrow) { color: var(--text-secondary); max-width: 43rem; }
  @media (max-width: 800px) { .hero { grid-template-columns: 1fr; } .hero-aside { display: none; } }
  @media (max-width: 640px) { .hero { padding-block: var(--space-5); } .platforms, .approach { grid-template-columns: 1fr; } .platforms { gap: var(--space-2); } .client-preview { margin-bottom: var(--space-5); } }
</style>
