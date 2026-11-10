<script lang="ts">
  import { page } from '$app/state';

  const links = [
    { href: '/showcase/', label: 'Showcase' },
    { href: '/docs/', label: 'Docs' },
    { href: '/about/', label: 'About' },
    { href: '/download/', label: 'Download' }
  ];
</script>

<a class="skip-link" href="#main-content">Skip to content</a>
<header class="site-header container">
  <a class="brand" href="/" aria-label="Screenroom home" aria-current={page.url.pathname === '/' ? 'page' : undefined}>
    <img class="brand-mark" src="/favicon.svg" alt="" width="28" height="28" />
    Screenroom
  </a>
  <nav aria-label="Main">
    {#each links as link}
      <a href={link.href} aria-current={page.url.pathname.startsWith(link.href) ? 'page' : undefined}>{link.label}</a>
    {/each}
  </nav>
</header>

<style>
  .skip-link { position: absolute; z-index: 2; top: -6rem; left: var(--gutter); padding: var(--space-2); background: var(--accent); color: var(--accent-ink); }
  .skip-link:focus { top: var(--space-2); }
  .site-header { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); padding-block: var(--space-3); border-bottom: 1px solid var(--border); }
  .brand { display: inline-flex; align-items: center; gap: 0.7rem; color: var(--text-primary); text-decoration: none; font-size: 1.25rem; font-weight: 600; letter-spacing: -0.04em; }
  .brand-mark { display: block; width: 1.75rem; height: 1.75rem; border-radius: var(--radius-small); }
  nav { display: flex; flex-wrap: wrap; gap: 0.25rem var(--space-4); }
  nav a { padding-block: 0.65rem; color: var(--text-secondary); font-size: var(--text-small); text-decoration: none; }
  nav a:hover, nav a[aria-current] { color: var(--text-primary); text-decoration: underline; text-decoration-color: var(--accent); text-underline-offset: 0.5em; }
  @media (max-width: 640px) {
    .site-header { align-items: flex-start; flex-direction: column; gap: var(--space-1); }
    nav { width: 100%; justify-content: space-between; gap: 0.5rem; }
  }
</style>
