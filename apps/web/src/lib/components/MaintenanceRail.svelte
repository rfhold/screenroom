<script lang="ts">
  let expanded = $state(false);
</script>

<aside class:expanded aria-label="Sidebar">
  <div class="rail-top"><button class="toggle" aria-label={expanded ? 'Collapse sidebar' : 'Expand sidebar'} aria-expanded={expanded} aria-controls="maintenance-menu" onclick={() => expanded = !expanded}>
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="3" /><path d="M9 4v16M5.5 8h1M5.5 12h1" /></svg>
  </button><span class="brand">Screenroom</span></div>
  <nav id="maintenance-menu" aria-label="Main menu">
    <a href="/" aria-label="Home"><svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="m3 10 9-7 9 7v10H15v-7H9v7H3Z" /></svg><span>Home</span></a>
    <a href="/?screen=search" aria-label="Search titles"><svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="10.5" cy="10.5" r="6.5" /><path d="m16 16 5 5" /></svg><span>Search titles</span></a>
    <a href="/?screen=browse&amp;library=all" aria-label="Libraries"><svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><rect x="4" y="4" width="16" height="16" rx="2" /><path d="M4 9h16M9 9v11" /></svg><span>Libraries</span></a>
    <a href="/?screen=maintenance" aria-label="Maintenance" aria-current="page"><svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="m4 20 8-8m-5-8 3 3-3 3-3-3a6 6 0 0 0 8 7l6 6a2 2 0 0 0 3-3l-6-6a6 6 0 0 0-8-7Z" /></svg><span>Maintenance</span></a>
  </nav>
</aside>

<style>
  aside { position: sticky; top: 0; align-self: flex-start; height: 100svh; width: 64px; min-width: 64px; background: var(--surface); border-right: 1px solid var(--border); padding: 16px 10px; }
  aside.expanded { width: 240px; min-width: 240px; }
  .rail-top { height: 48px; display: flex; align-items: start; gap: 12px; }
  .brand { display: none; font-size: 14px; font-weight: 600; padding-top: 13px; }
  .toggle { display: flex; align-items: center; justify-content: center; width: 44px; height: 44px; padding: 10px; background: transparent; border-color: transparent; }
  svg { width: 22px; height: 22px; stroke: currentColor; stroke-width: 1.5; flex: 0 0 22px; stroke-linejoin: round; stroke-linecap: round; }
  nav { display: grid; gap: 6px; margin-top: 16px; }
  nav a { display: flex; align-items: center; gap: 12px; min-height: 44px; padding: 11px; color: var(--muted); text-decoration: none; border-radius: 8px; font-size: 14px; }
  nav a:hover, nav a[aria-current] { background: #ffffff0c; color: var(--text); }
  nav span { display: none; }
  .expanded nav span, .expanded .brand { display: inline; }
  @media (max-width: 600px) {
    aside, aside.expanded { position: static; align-self: stretch; width: 100%; min-width: 0; height: auto; border-right: 0; border-bottom: 1px solid var(--border); padding: 10px 16px; }
    aside:not(.expanded) { display: flex; align-items: start; gap: 12px; }
    .rail-top { height: 44px; }
    aside:not(.expanded) nav { display: flex; gap: 2px; margin: 0; }
    .expanded nav { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    nav a { padding: 11px; }
  }
</style>
