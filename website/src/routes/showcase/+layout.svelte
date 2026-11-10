<script lang="ts">
  let { children } = $props();
</script>

<div class="showcase container">{@render children()}</div>

<style>
  .showcase { padding-block: var(--space-6); overflow-wrap: anywhere; }
  .showcase :global(header) { max-width: 48rem; margin-bottom: var(--space-5); }
  .showcase :global(header p:not(.eyebrow)) { color: var(--text-secondary); }
  .showcase :global(h1) { font-size: clamp(2.75rem, 6vw, 4.75rem); }
  .showcase :global(h2) { font-size: 1.65rem; }
  .showcase :global(.preview-note) { border-left: 2px solid var(--accent); padding-left: var(--space-3); }
  .showcase :global(.shell-grid) { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 17rem), 1fr)); gap: var(--space-3); padding: 0; list-style: none; }
  .showcase :global(.shell-card) { min-width: 0; padding: var(--space-3); border: 1px solid var(--border); border-radius: var(--radius); background: var(--surface-raised); }
  .showcase :global(.shell-card h2) { font-size: 1.25rem; margin-bottom: var(--space-1); }
  .showcase :global(.shell-card h2 a) { color: var(--text-primary); }
  .showcase :global(.shell-card p) { margin: var(--space-1) 0 0; color: var(--text-secondary); font-size: var(--text-small); }
  .showcase :global(.thumbnail) { display: flex; align-items: center; justify-content: center; aspect-ratio: 16 / 10; margin-bottom: var(--space-3); border-radius: var(--radius-small); background: var(--surface-inset); }
  .showcase :global(.thumbnail img) { display: block; width: 100%; height: 100%; object-fit: contain; }
  .showcase :global(.flows) { display: grid; gap: var(--space-6); }
  .showcase :global(section), .showcase :global(figure) { scroll-margin-top: var(--space-4); }
  .showcase :global(.flows section) { padding-top: var(--space-4); border-top: 1px solid var(--border); }
  .showcase :global(.flows section > p) { color: var(--text-secondary); }
  .showcase :global(h3) { margin-bottom: var(--space-3); }
  .showcase :global(.flow-nav) { margin-bottom: var(--space-5); padding: var(--space-2) var(--space-3); border: 1px solid var(--border); border-radius: var(--radius); background: var(--surface-raised); }
  .showcase :global(.flow-nav ul), .showcase :global(.viewport-nav ul), .showcase :global(.shell-nav ul) { display: flex; flex-wrap: wrap; gap: var(--space-1) var(--space-3); margin: 0; padding: 0; list-style: none; }
  .showcase :global(.flow-nav a), .showcase :global(.viewport-nav a), .showcase :global(.shell-nav a) { display: inline-block; padding-block: var(--space-1); }
  .showcase :global(.viewport-nav) { margin-bottom: var(--space-3); }
  .showcase :global(.captures) { display: grid; gap: var(--space-4); }
  .showcase :global(figure) { min-width: 0; margin: 0; }
  .showcase :global(.capture-frame) { display: flex; justify-content: center; padding: clamp(0.5rem, 2vw, 1.5rem); border: 1px solid var(--border); border-radius: var(--radius); background: var(--surface-inset); }
  .showcase :global(.capture-frame img) { display: block; width: auto; height: auto; max-width: 100%; max-height: 80vh; object-fit: contain; }
  .showcase :global(figcaption) { margin-top: var(--space-2); color: var(--text-secondary); font-size: var(--text-small); }
  .showcase :global(.shell-nav) { margin-top: var(--space-5); border-top: 1px solid var(--border); padding-top: var(--space-3); }
  .showcase :global([aria-current='page']) { color: var(--text-primary); font-weight: 600; }
  @media (max-width: 640px) { .showcase { padding-block: var(--space-5); } }
</style>
