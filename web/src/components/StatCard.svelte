<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { Health } from '../lib/api/types';
  import HealthBadge from './HealthBadge.svelte';

  let {
    title,
    subtitle,
    health,
    wide = false,
    children,
  }: {
    title: string;
    subtitle?: string;
    /** Omitted for cards whose metric has no health of its own (e.g. host info). */
    health?: Health;
    /** Spans two grid columns where there is room. */
    wide?: boolean;
    children: Snippet;
  } = $props();

  const id = $props.id();
</script>

<section class="card" class:wide aria-labelledby={id} data-health={health}>
  <header class="head">
    <div class="titles">
      <h3 class="title" {id}>{title}</h3>
      {#if subtitle}<p class="subtitle">{subtitle}</p>{/if}
    </div>
    {#if health}<HealthBadge {health} size="sm" />{/if}
  </header>
  {@render children()}
</section>

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
    padding: var(--space-4);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-sm);
  }

  /* Non-healthy cards get a colored edge so problems stand out when scanning the grid. */
  .card[data-health='warning'] {
    border-color: var(--health-warning-border);
  }

  .card[data-health='critical'] {
    border-color: var(--health-critical-border);
  }

  @media (min-width: 40rem) {
    .wide {
      grid-column: span 2;
    }
  }

  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .titles {
    min-width: 0;
  }

  .title {
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }

  .subtitle {
    font-size: var(--text-xs);
    color: var(--color-text-faint);
  }
</style>
