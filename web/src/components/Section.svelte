<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    title,
    count,
    children,
  }: {
    title: string;
    /** Shown after the title, e.g. the number of configured services. */
    count?: number;
    children: Snippet;
  } = $props();

  const id = $props.id();
</script>

<section class="section" aria-labelledby={id}>
  <h2 class="heading" {id}>
    {title}
    {#if count !== undefined}<span class="count">{count}</span>{/if}
  </h2>
  {@render children()}
</section>

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }

  .heading {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    font-size: var(--text-md);
  }

  .count {
    font-size: var(--text-sm);
    font-weight: var(--weight-regular);
    color: var(--color-text-faint);
  }
</style>
