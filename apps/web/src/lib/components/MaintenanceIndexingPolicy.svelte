<script lang="ts">
  import type { OperatorIndexingConfiguration, OperatorLibrary, OperatorIndexingMode, OperatorIndexingStrategy } from '../../../../../target/client-core/wasm/screenroom_client_core';
  let { draft = $bindable(), libraries, disabled = false }: { draft: OperatorIndexingConfiguration; libraries: OperatorLibrary[]; disabled?: boolean } = $props();
  function mode(value: string) {
    draft.mode = value as OperatorIndexingMode;
    draft.components.metadata = value !== 'repair';
    if (value === 'repair') { draft.components.artwork = true; draft.components.embeddings = false; draft.strategy = 'missing'; }
    else { draft.strategy = 'all'; draft.scope.item_kinds = ['movie','series']; draft.scope.item_ids = []; }
    draft.artwork.posters.skip_existing = true; draft.stale_after_seconds = null;
  }
  function strategy(value: string) { draft.strategy = value as OperatorIndexingStrategy; draft.artwork.posters.skip_existing = value === 'missing'; draft.stale_after_seconds = value === 'stale' ? 86400 : null; }
  function budget(name: 'work_requests'|'lifetime_requests'|'lifetime_records', value: string) { draft.limits[name] = /^\d+$/.test(value) ? BigInt(value) : 0n; }
</script>

<fieldset {disabled}>
  <legend>Indexing configuration · changes apply only when saved</legend>
  <div class="grid">
    <label>Mode<select value={draft.mode} onchange={e => mode(e.currentTarget.value)}><option value="incremental">Incremental</option><option value="full">Full</option><option value="reconcile">Reconcile</option><option value="repair">Poster repair</option></select></label>
    <label>Selection strategy<select value={draft.strategy} onchange={e => strategy(e.currentTarget.value)} disabled={draft.mode !== 'repair'}>{#each draft.mode === 'repair' ? ['missing','changed','stale','failed','all'] : ['all'] as value}<option {value}>{value}</option>{/each}</select></label>
  </div>
  <p class="hint">{draft.mode === 'repair' ? 'Repair only posters, without a metadata traversal. Skip existing is on only for missing posters.' : 'Traverse metadata for both movies and series. Incremental may fall back to a full traversal; the run records its effective plan.'} Changing mode adjusts only this draft’s supported components, strategy and scope.</p>
  <div class="checks"><label><input type="checkbox" checked={draft.components.metadata} disabled />Metadata {draft.mode !== 'repair' ? '(required)' : '(not supported)'}</label><label><input type="checkbox" bind:checked={draft.components.artwork} disabled={draft.mode === 'repair'} />Artwork · posters</label><label><input type="checkbox" bind:checked={draft.components.embeddings} disabled={draft.mode === 'repair'} />Embeddings</label></div>
  <p class="hint">Artwork → Posters → Skip existing: {String(draft.artwork.posters.skip_existing)}. Non-repair artwork uses revision-aware reuse.</p>
  {#if draft.strategy === 'stale'}<label>Stale poster age (seconds)<input type="number" min="60" max="2592000" step="1" required bind:value={draft.stale_after_seconds} /></label>{/if}
  <fieldset class="checks"><legend>Saved libraries</legend>{#each libraries as library}<label><input type="checkbox" bind:group={draft.scope.library_ids} value={library.public_id} />{library.title}</label>{/each}</fieldset>
  <fieldset class="checks" disabled={draft.mode !== 'repair'}><legend>Saved item kinds</legend>{#each ['movie','series'] as kind}<label><input type="checkbox" bind:group={draft.scope.item_kinds} value={kind} />{kind}</label>{/each}</fieldset>
  {#if draft.mode === 'repair'}<label>Repair item IDs (comma separated, optional)<textarea value={draft.scope.item_ids.join(', ')} oninput={e => draft.scope.item_ids = e.currentTarget.value.split(',').map(v => v.trim()).filter(Boolean)} rows="2"></textarea></label>{/if}
  <div class="checks"><label><input type="checkbox" bind:checked={draft.enabled} />Policy enabled</label><label><input type="checkbox" bind:checked={draft.automatic_runs} />Automatic runs</label></div>
  <p class="hint">Automatic runs are opt-in. The source must already be enrolled with one server-resolved local profile. No profile or provider credentials are configured here.</p>
  <details><summary>Schedule, budgets &amp; retry limits</summary>
    <div class="grid">
      <label>Cadence (seconds)<input type="number" min="60" max="2592000" step="1" required bind:value={draft.cadence_seconds} /></label>
      <label>Reconcile interval (seconds)<input type="number" min="60" max="2592000" step="1" required bind:value={draft.reconcile_seconds} /></label>
      <label>Incremental overlap (seconds)<input type="number" min="1" max="86400" step="1" required bind:value={draft.overlap_seconds} /></label>
      <label>Source concurrency<input type="number" value="1" disabled /></label>
      <label>Work request reservations<input type="number" min="7" max="1000" step="1" required value={draft.limits.work_requests.toString()} oninput={e => budget('work_requests',e.currentTarget.value)} /></label>
      <label>Work records<input type="number" min="3" max="1000" step="1" required bind:value={draft.limits.work_records} /></label>
      <label>Lifetime request reservations<input type="number" min="1" max="100000000" step="1" required value={draft.limits.lifetime_requests.toString()} oninput={e => budget('lifetime_requests',e.currentTarget.value)} /></label>
      <label>Lifetime records<input type="number" min="1" max="100000000" step="1" required value={draft.limits.lifetime_records.toString()} oninput={e => budget('lifetime_records',e.currentTarget.value)} /></label>
      <label>Run deadline (seconds)<input type="number" min="60" max="2592000" step="1" required bind:value={draft.limits.deadline_seconds} /></label>
      <label>Work timeout (seconds)<input type="number" min="1" max="240" step="1" required bind:value={draft.limits.work_timeout_seconds} /></label>
      <label>Maximum attempts<input type="number" min="1" max="20" step="1" required bind:value={draft.limits.retry.max_attempts} /></label>
      <label>Retry base (seconds)<input type="number" min="1" max="300" step="1" required bind:value={draft.limits.retry.base_seconds} /></label>
      <label>Retry cap (seconds)<input type="number" min={draft.limits.retry.base_seconds} max="86400" step="1" required bind:value={draft.limits.retry.cap_seconds} /></label>
      <label>Retry jitter (%)<input type="number" min="0" max="100" step="1" required bind:value={draft.limits.retry.jitter_percent} /></label>
    </div><p class="hint">Only concurrency 1 is supported. There is no proactive rate-control setting. Request budgets count reservations, not observed wire calls.</p>
  </details>
</fieldset>

<style>
  fieldset { border:0; padding:0; margin:16px 0; min-width:0; } legend,.hint { color:var(--muted); font-size:12px; line-height:1.6; } .grid { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:16px; } label { display:grid; gap:8px; font-size:12px; min-width:0; margin:8px 0; } input,select,textarea { width:100%; min-width:0; min-height:44px; background:var(--bg); color:var(--text); border:1px solid var(--border); border-radius:8px; padding:10px 12px; font:inherit; } .checks { display:flex; flex-wrap:wrap; gap:4px 20px; } .checks label { display:flex; align-items:center; min-height:44px; margin:0; overflow-wrap:anywhere; } input[type=checkbox] { width:16px; min-height:16px; flex-shrink:0; } details { border-top:1px solid var(--border); margin-top:20px; } summary { min-height:44px; padding:14px 0; cursor:pointer; font-size:13px; } :disabled { opacity:.6; } @media(max-width:600px) { .grid { grid-template-columns:minmax(0,1fr); gap:4px; } }
</style>
