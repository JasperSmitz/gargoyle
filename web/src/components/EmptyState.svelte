<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    title,
    heading = false,
    children,
  }: {
    title: string;
    /** The page's own heading (a page that is nothing but this state, e.g. not found). */
    heading?: boolean;
    children?: Snippet;
  } = $props();
</script>

<div class="empty">
  <svelte:element this={heading ? 'h1' : 'p'} class="title">{title}</svelte:element>
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
    font-size: var(--text-md);
    font-weight: var(--weight-medium);
  }

  .hint {
    font-size: var(--text-sm);
    color: var(--color-text-muted);
  }
</style>
