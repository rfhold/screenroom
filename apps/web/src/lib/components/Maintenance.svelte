<script lang="ts">
  import MaintenanceIndexing from './MaintenanceIndexing.svelte';
</script>

<svelte:head><title>Maintenance · Screenroom</title></svelte:head>
<div class="maintenance">
  <section class="content">
    <header><p class="eyebrow">Library care</p><h1>Maintenance</h1><p class="subtitle">Keep your library up to date.</p></header>
    <MaintenanceIndexing />
  </section>
</div>

<style>
  .content { min-width:0;width:100%;max-width:1328px;margin:0 auto;padding:20px 16px 24px; } header { margin-bottom:30px; } .eyebrow { margin:0 0 8px;font-size:11px;font-weight:600;letter-spacing:1.2px;text-transform:uppercase;color:var(--muted); } .subtitle { margin:10px 0 0;color:var(--muted);font-size:14px; } @media(max-width:600px) { .content { padding:8px 0; } }
</style>
