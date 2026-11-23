<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { newClient } from '../core/client';
  import { collectPages } from '../core/pagination';
  import { MetadataOperation, MetadataCancelled } from '../core/operation.mjs';
  import { changeJellyfinLink } from '../core/jellyfin.mjs';
  import { revokePlayback } from '../core/playback-lifecycle';
  import type { Client, JellyfinSource, JellyfinProfile, JellyfinLink } from '../../../../../target/client-core/wasm/screenroom_client_core';
  let { onChanged }: { onChanged: () => void } = $props();
  let sources = $state<JellyfinSource[]>([]);
  let profiles = $state<JellyfinProfile[]>([]);
  let source = $state('');
  let selected = $state('');
  let link = $state<JellyfinLink | null>(null);
  let known = $state(false);
  let busy = $state(false);
  let error = $state('');
  let status = $state('');
  let generation = 0;
  let operation: MetadataOperation | undefined;
  let connection: Client | undefined;
  function begin() {
    operation?.cancel();
    connection?.cancel_all();
    const current = ++generation;
    operation = new MetadataOperation(() => generation === current);
    busy = true; error = ''; status = '';
    return operation;
  }
  async function load(target = source) {
    const op = begin();
    known = false; profiles = [];
    try {
      const c = connection ??= await op.read(newClient);
      c.invalidate_reads();
      sources = await collectPages(async page => {
        const response = await c.list_jellyfin_sources(page);
        return { entries: response.sources, next_page_token: response.next_page_token };
      }, undefined, entry => entry.id, op);
      source = sources.some(entry => entry.id === target) ? target : sources[0]?.id ?? '';
      if (source) {
        const id = source;
        profiles = await collectPages(async page => {
          const response = await c.list_jellyfin_profiles(id, page);
          return { entries: response.profiles, next_page_token: response.next_page_token };
        }, undefined, entry => entry.id, op);
        link = await op.read(() => c.get_jellyfin_link(id));
        selected = link?.jellyfin_user_id ?? profiles[0]?.id ?? '';
        known = true;
      }
    } catch (cause) {
      if (!(cause instanceof MetadataCancelled) && op === operation) {
        connection?.cancel_all();
        error = 'Unable to read Jellyfin profile settings. Retry to read the server state.';
      }
    } finally { if (op === operation) busy = false; }
  }
  async function change(profile: string | null) {
    if (!connection || !known || busy) return;
    revokePlayback();
    const op = begin();
    known = false;
    try {
      const result = await changeJellyfinLink(connection, source, profile, op);
      op.check();
      link = result.link; known = true;
      selected = link?.jellyfin_user_id ?? profiles[0]?.id ?? '';
      if (result.confirmed) {
        status = result.ambiguous ? 'Server state checked: profile change is saved.' : 'Profile change saved on the server.';
        onChanged();
      } else {
        error = 'The requested change was not confirmed. Current server state is shown; choose a profile before trying again.';
        onChanged();
      }
    } catch (cause) {
      if (!(cause instanceof MetadataCancelled) && op === operation) {
        connection?.cancel_all();
        error = 'Profile change could not be confirmed. Retry reads server state only; the change will not be repeated.';
      }
    } finally { if (op === operation) busy = false; }
  }
  $effect(() => { untrack(() => { void load(); }); });
  onDestroy(() => { generation++; operation?.cancel(); connection?.cancel_all(); });
</script>

<section aria-labelledby="jellyfin-profile-title" aria-busy={busy}>
  <h2 id="jellyfin-profile-title">Jellyfin profile</h2>
  <p>Server-saved progress mapping for this local user selector. This is not sign-in.</p>
  {#if busy}<p role="status">Loading Jellyfin profile settings…</p>{/if}
  {#if error}<p role="alert">{error}</p><button disabled={busy} onclick={() => load()}>Retry Jellyfin settings</button>{/if}
  {#if status}<p role="status">{status}</p>{/if}
  {#if sources.length}
    <label>Jellyfin source<select value={source} disabled={busy} onchange={event => load(event.currentTarget.value)}>{#each sources as entry (entry.id)}<option value={entry.id}>{entry.display_name}</option>{/each}</select></label>
    <p>Current profile: {known ? link ? profiles.find(entry => entry.id === link?.jellyfin_user_id)?.display_name ?? 'Linked profile unavailable in choices' : 'Not linked' : 'Not confirmed'}</p>
    <label>Profile<select bind:value={selected} disabled={busy || !known || !profiles.length}>{#each profiles as entry (entry.id)}<option value={entry.id}>{entry.display_name}</option>{/each}</select></label>
    {#if known && !profiles.length}<p>No enabled profiles are available.</p>{/if}
    <button disabled={busy || !known || !profiles.some(entry => entry.id === selected)} onclick={() => change(selected)}>Save Jellyfin profile</button>
    <button disabled={busy || !known || !link} onclick={() => change(null)}>Unlink Jellyfin profile</button>
  {:else if !busy && !error}<p>No Jellyfin sources are available.</p>{/if}
</section>

<style>
  section { border-top: 1px solid var(--border); margin-top: 24px; padding-top: 16px; }
  h2 { font-size: 14px; }
  p, label, button, select { font-size: 12px; }
  p { color: var(--muted); }
  label { display: grid; gap: 6px; margin: 12px 0; }
  select { width: 100%; min-width: 0; }
  button { margin: 6px 0; }
</style>
