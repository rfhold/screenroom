<script lang="ts">
  import { operatorConfig } from '../core/operator-config';
  import MaintenanceRail from './MaintenanceRail.svelte';
  import MaintenanceIndexing from './MaintenanceIndexing.svelte';
</script>

<svelte:head><title>Maintenance · Screenroom</title></svelte:head>
<a class="skip-link" href="#main">Skip to content</a>
<div class="maintenance">
  <MaintenanceRail />
  <main id="main" tabindex="-1">
    <header><p class="eyebrow">Library care</p><h1>Maintenance</h1><p class="subtitle">Keep your library up to date.</p></header>
    {#if operatorConfig.enabled}<MaintenanceIndexing />{:else}<section class="empty"><p>Maintenance is unavailable on this connection.</p></section>{/if}
  </main>
</div>

<style>
  .skip-link { position:absolute;top:8px;left:16px;padding:12px;background:var(--surface);color:var(--text);transform:translateY(-200%);z-index:10; } .skip-link:focus { transform:none; } .maintenance { display:flex;min-height:100svh; } main { min-width:0;width:100%;max-width:1328px;margin:0 auto;padding:44px 48px 64px; } header { margin-bottom:30px; } .eyebrow { margin:0 0 8px;font-size:11px;font-weight:600;letter-spacing:1.2px;text-transform:uppercase;color:var(--muted); } .subtitle { margin:10px 0 0;color:var(--muted);font-size:14px; } .empty { border:1px solid var(--border);border-radius:12px;background:var(--surface);padding:48px 24px;text-align:center;color:var(--muted); } @media(max-width:1000px) { main { padding:32px 28px 48px; } } @media(max-width:600px) { .maintenance { flex-direction:column; } main { padding:28px 16px 40px; } }
</style>
