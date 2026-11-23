<script lang="ts">
  import { onMount } from 'svelte';
  import type { OperatorClient, OperatorConnection, OperatorConnectionDiscovery, OperatorSaveConnectionRequest, OperatorSavedConnection, OperatorFailure } from '../../../../../target/client-core/wasm/screenroom_client_core';
  import { newOperatorClient, ReadOwner } from '../core/operator';
  import { OwnedReads } from '../core/owned-reads';
  import { SessionAction, operatorError } from '../core/operator-state';
  import { selectionError, connectionUrlError } from '../core/connection-selection';
  import { webConfig } from '../core/config';
  let { onSaved }: {onSaved:(result:OperatorSavedConnection)=>void} = $props();
  let client:OperatorClient|undefined, reads:OwnedReads|undefined;
  const owner = new ReadOwner(), listOwner = new ReadOwner();
  const { user_id: localUser } = webConfig();
  const configuredUser = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(localUser);
  let connections = $state<OperatorConnection[]>([]), after = $state<string|null>(null), listing = $state(false), listError = $state('');
  let opened = $state(false), url = $state(''), key = $state(''), discovery = $state<OperatorConnectionDiscovery|null>(null), profile = $state(''), libraries = $state<string[]>([]);
  let busy = $state(false), error = $state(''), notice = $state(''), pending = $state<SessionAction<OperatorSaveConnectionRequest>|null>(null), uncertain = $state(false);
  const validation = $derived(selectionError(discovery,profile,libraries));
  const request = <T,>(start:()=>Promise<T>) => reads ? reads.read(start) : Promise.reject(new Error('Unavailable'));
  async function load(more = false) {
    if (!client || listing) return;
    listing = true; const ticket = listOwner.ticket(), api = client; listError = '';
    try { const result = await request(() => api.connections({after:more ? after : null,limit:100})); if (ticket.current()) { connections = more ? [...connections,...result.items] : result.items; after = result.next_after; } }
    catch(e) { if (ticket.current()) listError = operatorError(e); }
    finally { if (ticket.current()) listing = false; }
  }
  function invalidate() { owner.reset(); client?.cancel_all(); discovery = null; profile = ''; libraries = []; error = ''; busy = false; }
  function cancel() { invalidate(); pending = null; uncertain = false; key = ''; url = ''; opened = false; notice = 'Setup closed. If a save was in progress, reload the server list to check whether it completed.'; }
  async function connect() {
    if (!client || busy || pending || !configuredUser) return;
    error = connectionUrlError(url); if (error) return;
    if (!key.trim()) { error = 'Enter an administrator API key.'; return; }
    invalidate(); const ticket = owner.ticket(), api = client, body = {base_url:url.trim(),api_key:key}; busy = true; notice = '';
    try { const result = await request(() => api.discover_connection(body)); if (ticket.current()) { discovery = result; profile = ''; libraries = []; } }
    catch(e) { if (ticket.current()) error = operatorError(e); }
    finally { if (ticket.current()) busy = false; }
  }
  async function save(refresh:boolean) {
    if (!client || busy || pending || validation || !discovery || !configuredUser) return;
    pending = new SessionAction({request_id:crypto.randomUUID(),base_url:url.trim(),api_key:key,server_id:discovery.server.id,profile_id:profile,library_ids:[...libraries],local_user_id:localUser,refresh});
    await send();
  }
  async function send() {
    if (!client || !pending || busy) return;
    const action = pending, ticket = owner.ticket(), api = client; busy = true; error = ''; notice = ''; uncertain = false;
    try { const result = await action.send(body => request(() => api.save_connection(body))); if (!ticket.current() || !result) return; pending = null; key = ''; discovery = null; profile = ''; libraries = []; opened = false; notice = result.run_id ? 'Server saved. Refresh requested.' : 'Server saved. Refresh whenever you’re ready.'; void load(); onSaved(result); }
    catch(e) { if (ticket.current()) { uncertain = (e as OperatorFailure).ambiguous !== false; if (!uncertain) pending = null; error = uncertain ? 'Could not confirm the save. Retry sends the same selection, or reload the server list to check its current state.' : operatorError(e); } }
    finally { if (ticket.current()) busy = false; }
  }
  onMount(() => {
    let disposed = false;
    void newOperatorClient().then(api => { if (disposed) { api.free(); return; } client = api; reads = new OwnedReads(api); void load(); }).catch(e => listError = operatorError(e));
    return () => { disposed = true; owner.reset(); listOwner.reset(); pending = null; key = ''; discovery = null; reads?.dispose(); };
  });
</script>

<section class="connections" aria-labelledby="servers-title">
  <div class="heading"><div><h2 id="servers-title">Your servers</h2><p>Connect Jellyfin to bring your own library into Screenroom.</p></div><button onclick={() => {opened = true; notice = '';}}>Add server</button></div>
  {#if listError}<p role="alert">{listError}</p>{/if}
  {#if connections.length}<ul>{#each connections as connection}<li><strong>{connection.display_name}</strong><span>Jellyfin {connection.server_version} · {connection.state}</span></li>{/each}</ul>{:else}<p class="hint">{listing ? 'Loading servers…' : 'No servers connected yet. Add your Jellyfin server to get started.'}</p>{/if}
  <div class="actions"><button onclick={() => load()} disabled={listing}>Reload servers</button>{#if after}<button onclick={() => load(true)} disabled={listing}>More servers</button>{/if}</div>
  {#if notice}<p role="status">{notice}</p>{/if}
  {#if opened}<div class="setup" aria-labelledby="setup-title"><h3 id="setup-title">Add Jellyfin server</h3>
    <p>Your administrator API key is sent only to Screenroom’s private backend for secure storage. It is never saved in this browser.</p>
    {#if !configuredUser}<p role="alert">This installation has no local user selector or it is invalid. Configure VITE_SCREENROOM_USER_ID before adding a server; a profile cannot be assigned without it.</p>{/if}
    <form onsubmit={e => {e.preventDefault();void connect();}}>
      <label>Server URL<input type="url" autocomplete="off" spellcheck="false" value={url} oninput={e => {url = e.currentTarget.value; invalidate();}} disabled={!!pending} placeholder="https://jellyfin.example.org" required /></label>
      {#if url.trim().startsWith('http:')}<p class="warning">HTTP is not encrypted. Use it only on a network you trust; HTTPS protects your key in transit.</p>{/if}
      <label>Administrator API key<input type="password" autocomplete="off" spellcheck="false" value={key} oninput={e => {key = e.currentTarget.value; invalidate();}} disabled={!!pending} required /></label>
      <p class="hint">Create an API key in the Jellyfin administrator dashboard. The URL must be reachable from Screenroom’s backend. In a container, localhost means that container, not your Jellyfin host.</p>
      <button type="submit" disabled={busy || !!pending || !configuredUser}>{busy && !pending ? 'Connecting…' : 'Connect'}</button>
    </form>
    {#if discovery}<div class="selection"><h3>{discovery.server.name}</h3><p>Jellyfin {discovery.server.version}</p>
      <label>Jellyfin profile<select bind:value={profile} disabled={busy || !!pending}><option value="">Choose a profile</option>{#each discovery.profiles as p}<option value={p.id}>{p.name}{p.hidden ? ' · hidden in Jellyfin' : ''}</option>{/each}</select></label>
      <p class="hint">This profile is used for installation-wide indexing and mapped to the current local user’s personal watched state. Other local users are unchanged.</p>
      <fieldset disabled={busy || !!pending}><legend>Libraries</legend><p class="hint">Choose 1–32 libraries, including at most one collections library. Unsupported types cannot be selected.</p>{#each discovery.libraries as library}<label class="check"><input type="checkbox" bind:group={libraries} value={library.id} disabled={!library.supported || library.kind === 'unsupported'} /><span>{library.name}<small>{library.kind}{!library.supported || library.kind === 'unsupported' ? ' · unsupported' : ''}</small></span></label>{/each}</fieldset>
      <p class="hint">Metadata and posters on · embeddings and automatic maintenance off. You can change saved settings later in Advanced.</p>
      {#if validation}<p class="hint" role="status">{validation}</p>{/if}
      <div class="actions"><button onclick={() => save(false)} disabled={busy || !!pending || !!validation || !configuredUser}>Save</button><button class="primary" onclick={() => save(true)} disabled={busy || !!pending || !!validation || !configuredUser}>Save and refresh</button></div>
    </div>{/if}
    {#if error}<p class="warning" role="alert">{error}</p>{/if}
    {#if uncertain}<button onclick={send} disabled={busy}>Retry save</button>{/if}
    <button class="cancel" onclick={cancel}>Cancel setup</button>
  </div>{/if}
</section>

<style>
  .connections{border:1px solid var(--border);border-radius:12px;background:var(--surface);padding:24px;margin-bottom:24px;min-width:0}.heading,.actions{display:flex;align-items:center;justify-content:space-between;gap:12px;flex-wrap:wrap}.actions{justify-content:flex-start}.heading h2{font-size:19px;margin:0}p{font-size:13px;color:var(--muted);line-height:1.7;overflow-wrap:anywhere}h3{font-size:17px}button,input,select{min-height:44px}button{font-size:13px}label{display:grid;gap:8px;font-size:13px;margin:16px 0}input:not([type=checkbox]),select{width:100%;min-width:0;box-sizing:border-box;padding:10px 12px;background:var(--bg);border:1px solid var(--border);border-radius:8px;color:var(--text);font:inherit}.setup{border-top:1px solid var(--border);margin-top:24px;padding-top:8px;max-width:720px}fieldset{border:0;padding:0;min-width:0;margin:24px 0}legend{font-size:14px;font-weight:600}.check{display:flex;align-items:center;gap:12px;min-height:44px;margin:4px 0}.check input{min-width:18px;margin:0}.check small{display:block;color:var(--muted);font-size:12px}.warning{color:#edb7a7}.cancel{margin-top:20px}.primary{background:#c4d7f5;color:#101010}ul{list-style:none;padding:0}li{display:flex;justify-content:space-between;gap:12px;flex-wrap:wrap;padding:16px 0;border-bottom:1px solid var(--border);font-size:13px;overflow-wrap:anywhere}li span{color:var(--muted)}@media(max-width:600px){.connections{padding:20px 16px}.heading{align-items:start}.actions>button{flex:1}}
</style>
