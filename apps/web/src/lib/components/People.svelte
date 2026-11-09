<script lang="ts">
  import type { PersonCredit } from '../core/models';
  import RemoteImage from './RemoteImage.svelte';
  let { title, people }: { title: string; people: PersonCredit[] } = $props();
  const initials = (name: string) => name.split(' ').map(n => n[0]).slice(0, 2).join('');
</script>

<section aria-label={title}>
  <h2>{title}</h2>
  {#if people.length}
    <ul>{#each people as person (`${person.id}-${person.role}`)}
      <li>
        {#if person.artwork}<div class="portrait"><RemoteImage art={person.artwork} /></div>
        {:else}<div class="initials" role="img" aria-label={`No portrait included for ${person.name}`}>{initials(person.name)}</div>{/if}
        <strong>{person.name}</strong><span>{person.role}</span>
        {#if person.artwork?.type === 'character still'}<small>Character still</small>{/if}
      </li>
    {/each}</ul>
  {:else}<p class="muted">No {title.toLowerCase()} credits listed in this snapshot.</p>{/if}
</section>

<style>
  section { margin-top: 30px; }
  h2 { font-size: 20px; }
  ul { list-style: none; display: grid; grid-template-columns: repeat(auto-fill, minmax(128px, 1fr)); gap: 20px 16px; padding: 0; margin: 0; }
  li { min-width: 0; }
  .portrait, .initials { width: 100%; aspect-ratio: 1; object-fit: cover; border-radius: 10px; background: #ffffff09; }
  .initials { display: grid; place-items: center; color: var(--muted); font-size: 28px; border: 1px solid var(--border); }
  strong, span, small { display: block; overflow-wrap: anywhere; }
  strong { font-size: 13px; line-height: 1.4; margin-top: 8px; min-height: 2.8em; }
  span { min-height: 3em; }
  span, small, p { color: var(--muted); font-size: 12px; line-height: 1.5; }
  small { margin-top: 4px; }
  @container (min-width: 640px) { ul { grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); } }
</style>
