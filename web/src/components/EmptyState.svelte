<script lang="ts">
  import type { Snippet } from 'svelte';

  let { title, children }: { title: string; children?: Snippet } = $props();
</script>

<div class="empty">
  <p class="title">{title}</p>
  {#if children}<p class="hint">{@render children()}</p>{/if}
</div>

<style>
  .empty {
    display: grid;
    gap: var(--space-1);
    padding: var(--space-5) var(--space-4);
    border: 1px dashed var(--color-border-strong);
    border-radius: var(--radius-lg);
    text-align: center;
  }

  .title {
    font-weight: var(--weight-medium);
  }

  .hint {
    font-size: var(--text-sm);
    color: var(--color-text-muted);
  }
</style>
