<script lang="ts">
  import { onMount } from 'svelte';
  import type { OperatorClient, OperatorSource, OperatorLibrary, OperatorIndexingPolicy, OperatorIndexingProgress, OperatorIndexingExecution, OperatorRun, OperatorRunStatus, OperatorScope, OperatorJob, OperatorAttempt, OperatorRedrive, OperatorQueueDiagnostics, OperatorFailure, OperatorIndexingConfigureRequest, OperatorStartRequest, OperatorRedriveRequest, OperatorCandidate } from '../../../../../target/client-core/wasm/screenroom_client_core';
  import { newOperatorClient, ReadOwner } from '../core/operator';
  import { OwnedReads } from '../core/owned-reads';
  import { collectPages } from '../core/pagination';
  import { clone, encode, SessionAction, operatorError } from '../core/operator-state';
  import { indexingDefaults, validateIndexing } from '../core/indexing-settings';
  import MaintenanceIndexingPolicy from './MaintenanceIndexingPolicy.svelte';
  import MaintenanceConnections from './MaintenanceConnections.svelte';
  type IndexingOperation = 'configure'|'start'|'redrive';
  type IndexingBody = OperatorIndexingConfigureRequest|OperatorStartRequest|OperatorRedriveRequest;
  type Pending = {operation:IndexingOperation;entity:string;action:SessionAction<IndexingBody>};
  let api: OperatorClient | undefined, owned: OwnedReads | undefined;
  const owner = new ReadOwner(), policyOwner = new ReadOwner(), runOwner = new ReadOwner(), statusOwner = new ReadOwner(), jobOwner = new ReadOwner(), queueOwner = new ReadOwner(), writeOwner = new ReadOwner();
  const candidateOwner = new ReadOwner();
  let candidates = $state<OperatorCandidate[]>([]), candidatesAfter = $state<string|null>(null), candidatesLoading = $state(false), candidateOpen = $state(false), pendingOnly = $state(false);
  const request = <T,>(start: () => Promise<T>): Promise<T> => owned ? owned.read(start) : Promise.reject(new Error('Maintenance is unavailable'));
  let sources = $state<OperatorSource[]>([]), sourceAfter = $state<string|null>(null), source = $state('');
  let libraries = $state<OperatorLibrary[]>([]), policies = $state<OperatorIndexingPolicy[]>([]), policiesAfter = $state<string|null>(null), policyId = $state('');
  let draft = $state(indexingDefaults()), manual = $state<OperatorScope|null>(null), progress = $state<OperatorIndexingProgress|null>(null);
  let runs = $state<OperatorRun[]>([]), runsAfter = $state<string|null>(null), runId = $state(''), detail = $state<OperatorRunStatus|null>(null), execution = $state<OperatorIndexingExecution|null>(null);
  let jobs = $state<OperatorJob[]>([]), jobsAfter = $state<string|null>(null), jobId = $state(''), attempts = $state<OperatorAttempt[]>([]), attemptsAfter = $state<string|null>(null), redrives = $state<OperatorRedrive[]>([]), redrivesAfter = $state<string|null>(null);
  let diagnostics = $state<OperatorQueueDiagnostics|null>(null), capabilities = $state('unknown'), capabilityList = $state(''), queueRun = $state(true);
  let pending = $state<Pending|null>(null);
  let error = $state(''), notice = $state(''), activeRun = $state(''), ready = $state(false), loading = $state(false), writing = $state(false), policiesLoading = $state(false), runsLoading = $state(false), jobsLoading = $state(false), historyLoading = $state(false), queueLoading = $state(false), editorOpen = $state(false), advanced = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const saved = $derived(policies.find(p => p.id === policyId)), job = $derived(jobs.find(j => j.id === jobId));
  const dirty = $derived(!saved || encode(saved.configuration) !== encode(draft));
  const validation = $derived.by(() => { try { validateIndexing(draft); return ''; } catch (e) { return (e as Error).message; } });
  const manualValid = $derived.by(() => { if (!saved || !manual) return false; try { validateIndexing({...clone(saved.configuration),scope:clone(manual)}); return manual.library_ids.every(id => saved.configuration.scope.library_ids.includes(id)) && manual.item_kinds.every(k => saved.configuration.scope.item_kinds.includes(k)) && (saved.configuration.scope.item_ids.length === 0 || (manual.item_ids.length > 0 && manual.item_ids.every(id => saved.configuration.scope.item_ids.includes(id)))); } catch { return false; } });
  const writable = $derived(ready && !writing && !pending);
  const canRefresh = $derived(writable && !!saved?.enabled && manualValid);
  const page = (after: string|null = null) => ({after,limit:100});
  const time = (s: string|null) => s ? new Date(s).toLocaleString() : 'Not reported';
  const text = (s: string) => s.replaceAll('_',' ');
  const count = (v: unknown) => v === null ? 'Unknown (capabilities unspecified)' : String(v);
  const short = (id: string) => `${id.slice(0,8)}…${id.slice(-4)}`;
  function report(cause: unknown) {
    const e = cause as Partial<OperatorFailure>;
    activeRun = e.run_id || '';
    error = e.ambiguous ? 'Could not confirm the request. Retry this action or reload to check the server state.' : operatorError(cause);
  }
  function stop() { clearTimeout(timer); runOwner.reset(); statusOwner.reset(); jobOwner.reset(); queueOwner.reset(); candidateOwner.reset(); candidates = []; candidatesAfter = null; candidatesLoading = false; jobsLoading = false; historyLoading = false; queueLoading = false; }
  async function loadSources(more = false) {
    if (!api || loading) return;
    const ticket = owner.ticket(); loading = true;
    try { const result = await request(() => api!.sources(page(more ? sourceAfter : null))); if (ticket.current()) { sources = more ? [...sources,...result.items] : result.items; sourceAfter = result.next_after; if (sources.length === 1 && !source) { source = sources[0].id; loading = false; void changeSource(); } } }
    catch(e) { if (ticket.current()) report(e); } finally { if (ticket.current()) loading = false; }
  }
  async function changeSource() {
    api?.cancel_all(); owner.reset(); policyOwner.reset(); writeOwner.reset(); stop();
    ready = false; writing = false; policiesLoading = false; runsLoading = false; loading = false; error = ''; notice = ''; activeRun = ''; policies = []; libraries = []; runs = []; jobs = []; attempts = []; redrives = []; pending = null; policyId = ''; runId = ''; jobId = ''; progress = null; detail = null; execution = null; diagnostics = null; manual = null; policiesAfter = null; runsAfter = null; jobsAfter = null; attemptsAfter = null; redrivesAfter = null;
    if (!api || !source) return;
    const ticket = owner.ticket(), selected = source, client = api; loading = true;
    try {
      const [ls,ps,rs] = await Promise.all([
        collectPages(async p => { const r = await request(() => client.libraries(selected,page(p.token || null))); return {entries:r.items.map(l => ({id:l.public_id,library:l})),next_page_token:r.next_after || ''}; },ticket.current),
        request(() => client.indexing_policies(selected,page())),request(() => client.indexing_runs(selected,page()))
      ]);
      if (!ticket.current()) return;
      libraries = ls.map(l => l.library); policies = ps.items; policiesAfter = ps.next_after; runs = rs.items; runsAfter = rs.next_after; ready = true; selectPolicy(ps.items[0]?.id || ''); if (rs.items[0]) void openRun(rs.items[0].id); if (advanced) void refreshQueue();
    } catch(e) { if (ticket.current()) report(e); } finally { if (ticket.current()) loading = false; }
  }
  function selectPolicy(id: string) {
    policyOwner.reset(); policyId = id; progress = null;
    const selected = policies.find(p => p.id === id); draft = selected ? clone(selected.configuration) : indexingDefaults(libraries); manual = selected ? clone(selected.configuration.scope) : null;
    if (selected && api) { const ticket = policyOwner.ticket(), selectedSource = source; void request(() => api!.indexing_progress(selectedSource,id)).then(p => { if (ticket.current()) progress = p; }).catch(e => { if (ticket.current()) report(e); }); }
  }
  async function loadPolicies(more = false) {
    if (!api || policiesLoading || writing) return; policiesLoading = true; const ticket = owner.ticket(), client = api, selected = source;
    try { const r = await request(() => client.indexing_policies(selected,page(more ? policiesAfter : null))); if (ticket.current()) { policies = more ? [...policies,...r.items] : r.items; policiesAfter = r.next_after; if (!more) selectPolicy(policies.find(p => p.id === policyId)?.id || policies[0]?.id || ''); } }
    catch(e) { if (ticket.current()) report(e); } finally { if (ticket.current()) policiesLoading = false; }
  }
  async function loadRuns(more = false) {
    if (!api || runsLoading) return; runsLoading = true; const ticket = owner.ticket(), selected = source, client = api;
    try { const r = await request(() => client.indexing_runs(selected,page(more ? runsAfter : null))); if (ticket.current()) { runs = more ? [...runs,...r.items] : r.items; runsAfter = r.next_after; } }
    catch(e) { if (ticket.current()) report(e); } finally { if (ticket.current()) runsLoading = false; }
  }
  async function mutate(operation: IndexingOperation, entity: string, body: IndexingBody) {
    if (!writable || !api) return;
    pending = {operation,entity,action:new SessionAction(body)};
    await perform(pending);
  }
  async function retryPending() {
    if (pending && !writing) await perform(pending);
  }
  async function perform(intent: Pending) {
    if (!api || writing || pending !== intent) return;
    writing = true; writeOwner.reset(); const ticket = writeOwner.ticket(), selected = source, client = api; error = ''; notice = ''; clearTimeout(timer);
    let open = '';
    try {
      const receipt = await intent.action.send<OperatorIndexingPolicy|OperatorRun|OperatorRedrive>(body => intent.operation === 'configure' ? request(() => client.indexing_configure(selected,body as OperatorIndexingConfigureRequest)) : intent.operation === 'start' ? request(() => client.indexing_start(selected,body as OperatorStartRequest)) : request(() => client.indexing_redrive(selected,intent.entity.split('/')[0],intent.entity.split('/')[1],body as OperatorRedriveRequest)));
      if (!ticket.current() || !receipt) return;
      pending = null;
      if (intent.operation === 'configure') { const p = receipt as OperatorIndexingPolicy; policies = [p,...policies.filter(old => old.id !== p.id)]; selectPolicy(p.id); notice = 'Settings saved.'; }
      if (intent.operation === 'start') { open = (receipt as OperatorRun).id; notice = 'Refresh requested.'; }
      if (intent.operation === 'redrive') { notice = 'Retry requested.'; void loadJobs(); if (jobId) void history(); if (advanced) void refreshQueue(); }
    } catch(e) {
      if (ticket.current()) {
        if ((e as OperatorFailure).ambiguous === false) pending = null;
        report(e);
      }
    } finally { if (ticket.current()) { writing = false; if (open) { void openRun(open); void loadRuns(); } else if (runId && !document.hidden) schedule(); } }
  }
  function save() { if (validation || !writable || policiesLoading) return; void mutate('configure',saved?.id || 'new',{request_id:crypto.randomUUID(),existing:saved ? {policy_id:saved.id,expected_version:saved.version} : null,configuration:clone(draft)}); }
  async function start() {
    if (!canRefresh || !api) return;
    if (saved) await mutate('start',saved.id,{request_id:crypto.randomUUID(),policy_id:saved.id,policy_version:saved.version,scope:clone(manual)});
  }
  function scheduleChange(automatic: boolean, cadence = saved?.configuration.cadence_seconds || 86400) {
    if (!writable || !saved) return; const configuration = clone(saved.configuration); configuration.automatic_runs = automatic; configuration.cadence_seconds = cadence;
    void mutate('configure',saved?.id || 'new',{request_id:crypto.randomUUID(),existing:saved ? {policy_id:saved.id,expected_version:saved.version} : null,configuration});
  }
  function schedule() { clearTimeout(timer); timer = setTimeout(() => { if (!document.hidden && !writing) void poll(); },10000); }
  async function poll() {
    if (!api || !runId || document.hidden || writing) return;
    statusOwner.reset(); const ticket = statusOwner.ticket(), selected = source, id = runId, client = api;
    try { const [d,e] = await Promise.all([request(() => client.indexing_run_status(selected,id)),request(() => client.indexing_execution(selected,id))]); if (ticket.current()) { detail = d; execution = e; } }
    catch(e) { if (ticket.current()) report(e); } finally { if (ticket.current() && !document.hidden && !writing) schedule(); }
  }
  async function openRun(id: string) {
    if (writing) return; stop(); runId = id; detail = null; execution = null; jobs = []; jobId = ''; jobsAfter = null; attempts = []; redrives = [];
    await Promise.all([poll(),loadJobs(),advanced ? refreshQueue() : null,candidateOpen ? loadCandidates() : null]);
  }
  async function loadCandidates(more = false) {
    if (!api || !runId || candidatesLoading) return;
    candidatesLoading = true; const ticket = candidateOwner.ticket(), client = api, selected = source, id = runId;
    try { const r = await request(() => client.indexing_candidates(selected,id,page(more ? candidatesAfter : null),pendingOnly)); if (ticket.current()) { candidates = more ? [...candidates,...r.items] : r.items; candidatesAfter = r.next_after; } }
    catch(e) { if (ticket.current()) report(e); } finally { if (ticket.current()) candidatesLoading = false; }
  }
  async function action(name: 'pause'|'resume'|'cancel') {
    if (!api || !writable || !detail?.allowed_actions.includes(name)) return;
    clearTimeout(timer); statusOwner.reset(); writing = true; writeOwner.reset(); const ticket = writeOwner.ticket(), client = api, selected = source, id = runId;
    try { const result = await request(() => client[`indexing_${name}`](selected,id)); if (ticket.current()) detail = result; }
    catch(e) { if (ticket.current()) { detail = null; report(e); } }
    finally { if (ticket.current()) { writing = false; void poll(); void loadRuns(); void refreshQueue(); } }
  }
  async function loadJobs(more = false) {
    if (!api || !runId || jobsLoading) return; jobsLoading = true; const ticket = runOwner.ticket(), client = api, selected = source, id = runId;
    try { const r = await request(() => client.indexing_jobs(selected,id,page(more ? jobsAfter : null))); if (ticket.current()) { jobs = more ? [...jobs,...r.items] : r.items; jobsAfter = r.next_after; if (jobId && !jobs.some(j => j.id === jobId)) jobId = ''; } }
    catch(e) { if (ticket.current()) report(e); } finally { if (ticket.current()) jobsLoading = false; }
  }
  function selectJob(id: string) { jobOwner.reset(); jobId = id; attempts = []; redrives = []; attemptsAfter = null; redrivesAfter = null; historyLoading = false; void history(); }
  async function history(which: 'both'|'attempts'|'redrives' = 'both') {
    if (!api || !jobId || historyLoading) return; historyLoading = true;
    const ticket = jobOwner.ticket(), client = api, selected = source, run = runId, id = jobId;
    try {
      const [a,r] = await Promise.all([which !== 'redrives' ? request(() => client.indexing_attempts(selected,run,id,page(which === 'attempts' ? attemptsAfter : null))) : null,which !== 'attempts' ? request(() => client.indexing_redrives(selected,run,id,page(which === 'redrives' ? redrivesAfter : null))) : null]);
      if (ticket.current()) { if(a) { attempts = which === 'attempts' ? [...attempts,...a.items] : a.items; attemptsAfter = a.next_after; } if(r) { redrives = which === 'redrives' ? [...redrives,...r.items] : r.items; redrivesAfter = r.next_after; } }
    } catch(e) { if (ticket.current()) report(e); } finally { if (ticket.current()) historyLoading = false; }
  }
  function redrive(id: string) { const selected = jobs.find(j => j.id === id); if (!selected?.redrive_allowed || !writable) return; void mutate('redrive',`${runId}/${selected.id}`,{request_id:crypto.randomUUID()}); }
  async function refreshQueue() {
    if (!api || !ready) return; queueOwner.reset(); const ticket = queueOwner.ticket(), client = api; queueLoading = true;
    const caps = capabilities === 'unknown' ? null : capabilities === 'none' ? [] : capabilityList.split(',').map(v => v.trim()).filter(Boolean);
    try { const r = await request(() => client.indexing_queue({run_id:queueRun && runId ? runId : null,capabilities:caps})); if (ticket.current()) diagnostics = r; }
    catch(e) { if (ticket.current()) report(e); } finally { if (ticket.current()) queueLoading = false; }
  }
  onMount(() => {
    let disposed = false;
    void newOperatorClient().then(client => { if (disposed) { client.free(); return; } api = client; owned = new OwnedReads(client); void loadSources(); }).catch(report);
    const visibility = () => { clearTimeout(timer); statusOwner.reset(); if (!document.hidden && runId && !writing) void poll(); };
    document.addEventListener('visibilitychange',visibility);
    return () => { disposed = true; owner.reset(); policyOwner.reset(); writeOwner.reset(); stop(); owned?.dispose(); document.removeEventListener('visibilitychange',visibility); };
  });
</script>

<div class="indexing">
  <MaintenanceConnections onSaved={async result => { await loadSources(); source = result.connection.source_id; await changeSource(); if (result.run_id) void openRun(result.run_id); }} />
  {#if sources.length || source}
  <div class="toolbar"><label>Catalog source<select bind:value={source} onchange={changeSource} disabled={writing}><option value="">Choose a source</option>{#each sources as s}<option value={s.id}>{s.source_key} · {s.provider} · {s.state}</option>{/each}</select></label><button onclick={() => source ? changeSource() : loadSources()} disabled={writing || loading}>Reload</button>{#if sourceAfter}<button onclick={() => loadSources(true)} disabled={loading}>More sources</button>{/if}</div>
  <p class="hint context">Refresh metadata and posters, or let Screenroom keep them up to date on a schedule.</p>
  {/if}
  {#if error}<div class="callout error" role="alert"><h2>Could not complete this action</h2><p>{error}</p>{#if activeRun}<button onclick={() => openRun(activeRun)} disabled={writing}>View current refresh</button>{/if}<a href="#maintenance-advanced" onclick={() => advanced = true}>More options</a></div>{/if}
  {#if notice}<p class="notice" role="status">{notice}</p>{/if}
  {#if loading}<p role="status" class="hint">Loading maintenance…</p>{/if}
  {#if !source && sources.length}<section class="panel empty"><h2>Choose a library source</h2><p>Refresh your library or set up automatic maintenance.</p></section>{/if}
  {#if source}
    {#if pending && !writing}<section class="callout" role="status"><h2>Could not confirm this action</h2><p>Retry sends the same request while this page is open. Reload checks the server’s current state without repeating it.</p><button onclick={retryPending}>Retry request</button></section>{/if}
  {/if}
  {#if source && ready}
    <section class="panel refresh-panel" aria-labelledby="refresh-title">
      <div class="section-heading"><div><p class="eyebrow">Your library</p><h2 id="refresh-title">Refresh library</h2><p class="plan">{saved?.configuration.mode === 'repair' ? 'Repair posters using your saved settings.' : saved?.configuration.components.artwork === false ? 'Update library metadata.' : 'Find new items and update metadata and posters.'}</p></div><button class="primary" onclick={start} disabled={!canRefresh}>{writing ? 'Working…' : 'Refresh'}</button></div>
      {#if policies.length > 1}<label>Refresh plan<select value={policyId} onchange={e => selectPolicy(e.currentTarget.value)} disabled={writing || !!pending}>{#each policies as p,index}<option value={p.id}>Plan {index + 1} · {text(p.configuration.mode)} · {p.configuration.components.artwork ? 'with posters' : 'metadata'}</option>{/each}</select></label>{/if}
      {#if !saved}<p class="hint">No saved refresh settings. Add a Jellyfin server above, or save settings in Advanced.</p>{:else if !saved.enabled}<p class="warning">This refresh plan is disabled. Enable it in Advanced settings to refresh.</p>{/if}
      <div class="schedule"><label class="check"><input type="checkbox" checked={saved?.automatic_runs || false} onchange={e => scheduleChange(e.currentTarget.checked)} disabled={!writable || !saved} />Automatic maintenance</label>{#if saved?.automatic_runs}<label>Refresh every<select value={saved.configuration.cadence_seconds} onchange={e => scheduleChange(true,Number(e.currentTarget.value))} disabled={!writable}><option value="3600">Hour</option><option value="86400">Day</option><option value="604800">Week</option>{#if ![3600,86400,604800].includes(saved.configuration.cadence_seconds)}<option value={saved.configuration.cadence_seconds}>Custom · {saved.configuration.cadence_seconds} seconds</option>{/if}</select></label><p class="hint">Next scheduled refresh: {saved.enabled ? time(saved.next_due_at) : 'paused while this plan is disabled'}.</p>{:else}<p class="hint">Off · refresh whenever you need it.</p>{/if}</div>
    </section>
    <section class="panel" aria-labelledby="last-refresh-title"><div class="section-heading"><div><h2 id="last-refresh-title">{runId ? 'Last refresh' : 'No refreshes yet'}</h2>{#if detail}<p>Started {time(detail.run.started_at)}</p>{/if}</div>{#if runId}<button onclick={() => openRun(runId)} disabled={writing}>Check progress</button>{/if}</div>
      {#if detail}<div class="verdict"><h3>{detail.all_complete ? 'Refresh complete' : detail.run.state === 'paused' ? 'Refresh paused' : detail.run.state === 'cancelled' ? 'Refresh cancelled' : detail.work_settled || detail.enumeration_complete || detail.terminal_reason || detail.run.state === 'failed' ? 'Refresh incomplete' : ['queued','pending','ready'].includes(detail.run.state) ? 'Refresh queued' : 'Refreshing library'}</h3><p>{detail.observed_records === null ? 'Progress has not been reported yet.' : `${detail.observed_records} library records seen.`} {detail.terminal_reason ? `Stopped: ${text(detail.terminal_reason)}.` : ''}</p></div>
        <div class="result-notes">{#each [['Request outcomes still unconfirmed',detail.coverage.unresolved_attempts],['Items unavailable',detail.candidates.unavailable],['Poster failures',detail.candidates.failed],['Failed work',detail.queue.failed],['Posters awaiting publication',detail.coverage.pending_artwork_publications],['Operations still unresolved',detail.coverage.unresolved_operations],['Items omitted',detail.coverage.omissions],['Failed follow-up work',detail.coverage.followup_failed]] as [name,value]}{#if value !== 0n}<p class="warning">{name}: {String(value)}</p>{/if}{/each}</div>
        <div class="actions">{#each ['pause','resume','cancel'] as name}{#if detail.allowed_actions.includes(name)}<button class:danger={name === 'cancel'} onclick={() => action(name as 'pause'|'resume'|'cancel')} disabled={!writable}>{name === 'pause' ? 'Pause' : name === 'resume' ? 'Resume' : 'Cancel refresh'}</button>{/if}{/each}</div>
        {#each jobs.filter(j => j.redrive_allowed) as j}<div class="retry-work"><p>{j.kind === 'artwork' ? 'Poster work' : j.kind === 'embedding' ? 'Embedding work' : 'Library updates'} could not finish.</p><button onclick={() => redrive(j.id)} disabled={!writable} aria-label={`Retry ${j.kind === 'artwork' ? 'posters' : j.kind === 'embedding' ? 'embeddings' : 'library updates'}`}>Retry</button></div>{/each}
        <p class="hint">Last checked {time(detail.as_of)}. These are saved results; worker activity is not verified here.</p>
      {:else}<p class="hint">{runId ? 'Checking refresh progress…' : 'Refresh whenever you need an update.'}</p>{/if}
    </section>
  {/if}
  {#if source}<details class="panel advanced" id="maintenance-advanced" bind:open={advanced} ontoggle={(event) => { advanced = event.currentTarget.open; if (advanced && ready && !diagnostics) void refreshQueue(); }}><summary>Advanced settings</summary>
    {#if ready}<div class="workflow"><details class="editor" bind:open={editorOpen}><summary><strong>Refresh settings</strong><span class="hint">{dirty ? 'Unsaved changes' : 'Saved settings'}</span></summary>
      <div class="toolbar"><label>Saved plan<select value={policyId} onchange={e => selectPolicy(e.currentTarget.value)} disabled={!writable}><option value="">New plan</option>{#each policies as p,index}<option value={p.id}>Plan {index + 1} · {text(p.configuration.mode)}</option>{/each}</select></label><button onclick={() => loadPolicies()} disabled={writing || policiesLoading}>Reload settings</button>{#if policiesAfter}<button onclick={() => loadPolicies(true)} disabled={writing || policiesLoading}>More plans</button>{/if}</div>
      <form onsubmit={e => {e.preventDefault();save();}}><MaintenanceIndexingPolicy bind:draft {libraries} disabled={!writable} />{#if validation}<p class="unsaved">{validation}</p>{/if}<button type="submit" disabled={!writable || !!validation || policiesLoading}>Save settings</button></form>
    </details><div>{#if saved && manual}<h3>This refresh’s scope</h3><p class="hint">Refresh always uses the saved plan. Unsaved settings do not affect it.</p><fieldset class="scope" disabled={!writable}><legend>Narrow the saved scope</legend>{#each saved.configuration.scope.library_ids as id}<label><input type="checkbox" bind:group={manual.library_ids} value={id} />{libraries.find(l => l.public_id === id)?.title || id}</label>{/each}{#each saved.configuration.scope.item_kinds as kind}<label><input type="checkbox" bind:group={manual.item_kinds} value={kind} disabled={saved.configuration.mode !== 'repair'} />{kind}</label>{/each}{#if saved.configuration.mode === 'repair'}{#each saved.configuration.scope.item_ids as id}<label><input type="checkbox" bind:group={manual.item_ids} value={id} />{id}</label>{/each}{/if}</fieldset>{#if saved.configuration.mode === 'repair' && !saved.configuration.scope.item_ids.length}<label>Manual repair item IDs (comma separated, optional)<input value={manual.item_ids.join(', ')} oninput={e => manual!.item_ids = e.currentTarget.value.split(',').map(v => v.trim()).filter(Boolean)} disabled={!writable} /></label>{/if}<p class="hint">Policy {saved.id} · v{saved.version}. This is a saved scope, not a preview or a candidate total.</p><p class="hint">Watermark {time(progress?.watermark || null)} · Progress version {progress?.policy_version.toString() || 'not reported'} · Next reconcile {time(progress?.next_reconcile_at || null)}</p>{/if}</div></div>{/if}
    {#if saved}<details><summary>Saved configuration</summary><pre>{encode(saved.configuration)}</pre></details>{/if}
    {#if runId}<details bind:open={candidateOpen} ontoggle={e => {candidateOpen = e.currentTarget.open;if (candidateOpen && !candidates.length) void loadCandidates();}}><summary>Poster candidate records</summary><label class="check"><input type="checkbox" bind:checked={pendingOnly} onchange={() => {candidateOwner.reset();candidatesLoading = false;void loadCandidates();}} />Pending candidates only</label><p class="hint">{candidates.length} candidate records loaded. This list is not an estimate of total library progress.</p><div class="history-list">{#each candidates as c}<p><span class="mono">{c.source_item_id}</span> · {text(c.state)} · {c.code ? text(c.code) : 'Outcome unknown'}{c.selection_reason ? ` · ${text(c.selection_reason)}` : ''}</p>{/each}</div>{#if candidatesAfter}<button onclick={() => loadCandidates(true)} disabled={candidatesLoading}>More candidates</button>{/if}<button onclick={() => loadCandidates()} disabled={candidatesLoading}>Reload candidates</button></details>{/if}
    {#if ready}<section><div class="section-heading"><div><h3>Refresh history</h3><p>{runs.length} refreshes loaded</p></div><button onclick={() => loadRuns()} disabled={runsLoading}>Reload history</button></div><div class="run-list">{#each runs as r}<button class="run-row" onclick={() => openRun(r.id)} disabled={writing} title={r.id} aria-label={`View refresh ${time(r.started_at)}`}><span>{time(r.started_at)}</span><span class="badge">{text(r.state)}</span><span class="mono">{short(r.id)}</span></button>{/each}</div>{#if runsAfter}<button onclick={() => loadRuns(true)} disabled={runsLoading}>More refreshes</button>{/if}</section>{/if}
    {#if detail}<section class="execution"><h3>Technical progress</h3><p class="mono">Run {runId} · Policy {detail.run.policy_id} · v{detail.run.policy_version}</p><dl class="facts">{#each [['Observed records',detail.observed_records === null ? 'Unknown' : detail.observed_records],['Metadata writes',detail.run.items],['Reserved requests',detail.run.requests],['Enumeration complete',detail.enumeration_complete],['Work settled',detail.work_settled],['Work succeeded',detail.work_succeeded],['All complete',detail.all_complete]] as [name,value]}<div><dt>{name}</dt><dd>{String(value)}</dd></div>{/each}</dl><p class="hint">Metadata writes are not progress totals. Requests count budget reservations, not observed wire calls.</p>{#if execution}<p class="hint">Effective mode {execution.mode} · Lower watermark {time(execution.lower_watermark)} · Candidate watermark {time(execution.candidate_watermark)} · Settled {time(execution.settled_at)} · Fallback {execution.fallback_reason ? text(execution.fallback_reason) : 'none reported'}</p>{/if}<details><summary>Coverage, poster &amp; queue counters</summary>{#each [['Coverage',detail.coverage],['Poster candidates',detail.candidates],['Queue',detail.queue]] as [name,values]}<h3>{String(name)}</h3><dl class="facts">{#each Object.entries(values) as [k,v]}<div><dt>{text(k)}</dt><dd>{String(v)}</dd></div>{/each}</dl>{/each}</details></section>{/if}
    {#if runId}<section><div class="section-heading job-heading"><div><h3>Jobs &amp; attempts</h3><p>{jobs.length} jobs loaded</p></div><button onclick={() => loadJobs()} disabled={jobsLoading || writing}>Reload jobs</button></div><div class="job-list">{#each jobs as j}<button class="run-row" onclick={() => selectJob(j.id)} disabled={writing} aria-pressed={jobId === j.id} aria-label={`Inspect job ${j.id}`}><span class="mono">{short(j.id)}</span><span>{j.kind} · {j.state}</span><span>Generation {j.generation} · {j.attempts}/{j.max_attempts} attempts</span></button>{/each}</div>{#if jobsAfter}<button onclick={() => loadJobs(true)} disabled={jobsLoading}>More jobs</button>{/if}
      {#if job}<section class="job-detail"><h3>Job history</h3><p class="mono">{job.id}</p><p class="hint">Available {time(job.available_at)} · Lease until {time(job.lease_until)} · Retry {job.redrive_allowed ? 'allowed' : `unavailable: ${text(job.redrive_reason || 'not permitted by server')}`}.</p><p class="hint">Retry creates another job generation without refunding the run’s budgets or extending its deadline. Earlier attempts and retries are retained.</p>
        <h3>Attempts</h3>
        <!-- svelte-ignore a11y_no_noninteractive_tabindex (Keyboard access to the bounded history scroll region.) -->
        <div class="history-list" tabindex="0" role="region" aria-label="Job attempt history">{#each attempts as a}<p><strong>Fence {a.fence.toString()} · generation {a.generation}</strong><br />{text(a.outcome)}{a.error_code ? ` · ${a.error_code}` : ''} · {time(a.started_at)} → {time(a.finished_at)}</p>{/each}</div>{#if !attempts.length}<p class="hint">{historyLoading ? 'Loading attempts…' : 'No attempts reported.'}</p>{/if}{#if attemptsAfter}<button onclick={() => history('attempts')} disabled={historyLoading}>More attempts</button>{/if}
        <h3>Redrive history</h3>
        <!-- svelte-ignore a11y_no_noninteractive_tabindex (Keyboard access to the bounded history scroll region.) -->
        <div class="history-list" tabindex="0" role="region" aria-label="Job redrive history">{#each redrives as r}<p class="hint">Generation {r.generation} · <span class="mono">{r.request_id}</span></p>{/each}</div>{#if !redrives.length}<p class="hint">No redrives reported.</p>{/if}{#if redrivesAfter}<button onclick={() => history('redrives')} disabled={historyLoading}>More redrives</button>{/if}
      </section>{/if}
    </section>{/if}
    <section class="panel" aria-labelledby="queue-title"><div class="section-heading"><div><p class="eyebrow">Shared work queue</p><h2 id="queue-title">Queue diagnostics</h2></div><button onclick={refreshQueue} disabled={queueLoading}>Refresh queue</button></div>
      <div class="toolbar"><label>Capability perspective<select bind:value={capabilities} onchange={refreshQueue}><option value="unknown">Unspecified · relative counts unknown</option><option value="none">Empty · no capabilities</option><option value="listed">Explicit capabilities</option></select></label>{#if capabilities === 'listed'}<label>Capabilities (comma separated)<input bind:value={capabilityList} onchange={refreshQueue} placeholder="ingest, artwork, embedding" /></label>{/if}<label class="check"><input type="checkbox" bind:checked={queueRun} onchange={refreshQueue} disabled={!runId} />Selected run only</label></div>
      <p class="hint">{queueRun && runId ? 'Selected run snapshot.' : 'All sources and runs; this is not limited to the selected source.'} Capability perspective is a diagnostic filter, not worker discovery.</p>
      {#if diagnostics}<p class="warning">Non-atomic observations · worker liveness {diagnostics.worker_liveness}. Blocker categories overlap; do not add them together.</p><p class="hint">Queue observation {time(diagnostics.queue_observation.started_at)} → {time(diagnostics.queue_observation.finished_at)} · Blockers as of {time(diagnostics.blockers_as_of)}</p><p class="hint">Effective capabilities: {diagnostics.effective_capabilities === null ? 'unspecified (null)' : diagnostics.effective_capabilities.length ? diagnostics.effective_capabilities.join(', ') : 'none (empty set)'}</p>
        <dl class="stats">{#each [['Ready',diagnostics.queue.ready],['Delayed',diagnostics.queue.delayed],['Leased',diagnostics.queue.leased],['Failed',diagnostics.queue.failed]] as [name,value]}<div><dt>{name}</dt><dd>{String(value)}</dd></div>{/each}</dl>
        <dl class="facts">{#each Object.entries(diagnostics.queue).filter(([k]) => !['ready','delayed','leased','failed'].includes(k)) as [k,v]}<div><dt>{text(k)}</dt><dd>{k === 'oldest_ready_seconds' && v === null ? 'No ready age reported' : count(v)}</dd></div>{/each}</dl><h3>Eligibility blockers</h3><dl class="facts">{#each Object.entries(diagnostics.blockers).filter(([k]) => !['as_of','next_eligible_at'].includes(k)) as [k,v]}<div><dt>{text(k)}</dt><dd>{count(v)}</dd></div>{/each}</dl><p class="hint">Next eligible: {time(diagnostics.blockers.next_eligible_at)} · Blocker observation {time(diagnostics.blockers.as_of)}</p><p class="hint">Blocked: {diagnostics.count_semantics.blocked}<br />Exhausted: {diagnostics.count_semantics.exhausted}</p>
      {:else}<p class="hint">{queueLoading ? 'Loading queue observations…' : 'Queue observations not available.'}</p>{/if}
    </section>
  </details>{/if}
</div>

<style>
  .refresh-panel .plan { text-transform:none;font-size:13px; }
  .refresh-panel .primary { margin-top:0;min-width:120px; } .schedule { border-top:1px solid var(--border);margin-top:24px;padding-top:16px;display:flex;align-items:center;flex-wrap:wrap;gap:12px 24px; } .schedule > p { margin:0; } .schedule > label:not(.check) { min-width:150px; } .retry-work { display:flex;justify-content:space-between;align-items:center;gap:16px;border-top:1px solid var(--border);margin-top:20px;padding-top:12px;font-size:13px; } .result-notes { margin:16px 0; } .result-notes p { margin:6px 0; } .advanced > summary { font-size:15px;color:var(--text);padding:4px 0; } @media(max-width:600px) { .refresh-panel > .section-heading { align-items:stretch;flex-direction:column; } .refresh-panel .primary { width:100%;max-width:none; } .schedule { align-items:start;flex-direction:column;gap:8px; } }
  .history-list { max-height:320px; overflow-y:auto; overflow-wrap:anywhere; padding-right:8px; }
  .indexing { min-width:0; } .toolbar,.actions,.section-heading { display:flex; align-items:end; gap:12px; flex-wrap:wrap; } .toolbar label { flex:1; min-width:0; max-width:450px; } .section-heading { justify-content:space-between; align-items:center; margin-bottom:20px; } label { display:grid; gap:8px; font-size:12px; color:var(--muted); } select,input:not([type=checkbox]) { width:100%; min-width:0; min-height:44px; padding:10px 12px; color:var(--text); background:var(--surface); border:1px solid var(--border); border-radius:8px; font:inherit; } button { min-height:44px; font-size:13px; } p { overflow-wrap:anywhere; line-height:1.7; } h2 { margin:0; font-size:19px; } h3 { font-size:14px; margin:16px 0 10px; } .hint,.section-heading p { font-size:12px; color:var(--muted); line-height:1.7; } .context { margin:12px 0 24px; } .panel { min-width:0; background:var(--surface); border:1px solid var(--border); border-radius:12px; margin-top:24px; padding:24px; } .empty { padding:48px 24px; text-align:center; } .empty p { color:var(--muted); font-size:13px; } .eyebrow { font-size:11px; color:var(--muted); text-transform:uppercase; letter-spacing:1.2px; margin:0 0 6px; } .badge { font-size:11px; border:1px solid var(--border); border-radius:6px; padding:5px 8px; color:var(--muted); } .callout { padding:20px; margin:20px 0; border:1px solid var(--border); border-left:2px solid #a8c9ff70; background:#ffffff04; border-radius:8px; } .callout h2 { font-size:14px; } .callout p { font-size:12px; color:var(--muted); } .callout a { color:var(--text); } .error { border-color:#cf8f7b70; } .error h2 { color:#edb7a7; } .notice { font-size:13px; color:#c4d7f5; } .workflow { display:grid; grid-template-columns:minmax(0,1.15fr) minmax(0,1fr); gap:32px; border-top:1px solid var(--border); padding-top:24px; margin-top:20px; align-items:start; } .plan { font-size:16px; text-transform:capitalize; } .scope { border:0; padding:0; margin:20px 0; display:flex; flex-wrap:wrap; gap:4px 20px; min-width:0; } legend { color:var(--muted); font-size:12px; margin-bottom:8px; } .scope label,.check { display:flex; align-items:center; gap:10px; min-height:44px; overflow-wrap:anywhere; color:var(--text); } input[type=checkbox] { width:16px; height:16px; flex-shrink:0; margin:0; } .primary { color:#151719; background:#c4d7f5; border-color:#c4d7f5; font-weight:600; margin-top:16px; } .danger { color:#edb7a7; border-color:#cf8f7b60; } .actions .danger { margin-left:auto; } .unsaved,.warning { color:#e0c38a; font-size:12px; } .editor { border:1px solid var(--border); border-radius:8px; padding:0 16px; min-width:0; } .editor > summary { display:flex; flex-direction:column; gap:6px; padding:16px 0; } .editor strong { color:var(--text); font-weight:500; } summary { cursor:pointer; min-height:44px; font-size:13px; padding:14px 0; color:var(--muted); } details { margin:16px 0; } .editor form { border-top:1px solid var(--border); padding-bottom:16px; } .run-list,.job-list { display:grid; gap:4px; } .run-row { display:flex; width:100%; justify-content:space-between; align-items:center; gap:12px; background:transparent; border-color:transparent; border-bottom-color:var(--border); border-radius:0; padding:12px 0; font-size:12px; text-align:left; } .run-row > span { min-width:0; overflow-wrap:anywhere; } .run-row[aria-pressed=true] { color:#c4d7f5; } .mono { font-family:ui-monospace,monospace; overflow-wrap:anywhere; font-size:12px; } .verdict { padding:12px 20px; border-left:2px solid #c4d7f570; background:#ffffff04; } .verdict h3 { margin-top:0; } .verdict p { color:var(--muted); font-size:12px; margin-bottom:0; } dl { margin:20px 0; } .stats { display:grid; grid-template-columns:repeat(4,minmax(0,1fr)); gap:12px; } .stats > div { padding:16px; border:1px solid var(--border); border-radius:8px; } dt { font-size:12px; color:var(--muted); overflow-wrap:anywhere; } dd { margin:8px 0 0; font-size:24px; font-variant-numeric:tabular-nums; overflow-wrap:anywhere; } .facts { display:grid; grid-template-columns:repeat(3,minmax(0,1fr)); gap:18px; } .facts dd { font-size:14px; } .execution { border-top:1px solid var(--border); border-bottom:1px solid var(--border); padding:4px 0; margin:20px 0; } .execution p { font-size:12px; } .job-heading { border-top:1px solid var(--border); padding-top:16px; margin-top:24px; } .job-detail { margin-top:24px; padding-top:8px; border-top:1px solid var(--border); } .history-list p { font-size:12px; color:var(--muted); padding:12px 0; border-bottom:1px solid var(--border); } .history-list strong { color:var(--text); font-weight:500; } pre { white-space:pre-wrap; overflow-wrap:anywhere; font-size:11px; max-width:100%; } @media(max-width:1000px) { .workflow { grid-template-columns:minmax(0,1fr); gap:20px; } } @media(max-width:600px) { .panel { padding:20px 16px; } .toolbar { align-items:stretch; } .toolbar label { flex-basis:100%; max-width:none; } .section-heading { align-items:start; } .section-heading > div { min-width:0; } .section-heading > button { max-width:120px; } .run-row { flex-wrap:wrap; justify-content:start; } .run-row > span:last-child { flex-basis:100%; } .stats { grid-template-columns:repeat(2,minmax(0,1fr)); gap:8px; } .facts { grid-template-columns:repeat(2,minmax(0,1fr)); gap:16px; } .stats dd { font-size:22px; } .callout { padding:16px; } .actions .danger { margin-left:0; } }
</style>
