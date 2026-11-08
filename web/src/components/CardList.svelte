<script lang="ts">
  import type { Snippet } from 'svelte';

  // A bordered list of rows (TargetRow or similar `<li>` items).
  let { label, children }: { label?: string; children: Snippet } = $props();
</script>

<ul class="list" aria-label={label}>
  {@render children()}
</ul>

<style>
  .list {
    margin: 0;
    padding: 0;
    list-style: none;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-sm);
    overflow: hidden;
  }
</style>
