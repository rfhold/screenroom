<footer class="container">
  <div><a class="footer-brand" href="/">Screenroom</a><p>A quieter space for film and television.</p></div>
  <div class="footer-meta"><span>An independent project · Early development</span><a href="/about/">About the project <span aria-hidden="true">↗</span></a></div>
</footer>

<style>
  footer { display: flex; justify-content: space-between; gap: var(--space-4); padding-block: var(--space-5); border-top: 1px solid var(--border); color: var(--text-secondary); font-size: var(--text-small); }
  .footer-brand { color: var(--text-primary); font-size: 1.1rem; font-weight: 600; text-decoration: none; }
  p { margin: var(--space-1) 0 0; }
  .footer-meta { display: flex; flex-direction: column; align-items: flex-end; gap: var(--space-1); }
  @media (max-width: 640px) { footer { flex-direction: column; } .footer-meta { align-items: flex-start; } }
</style>
