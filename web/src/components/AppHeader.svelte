<script lang="ts">
  import type { Health } from '../lib/api/types';
  import { formatRelativeTime, formatTimestamp } from '../lib/format';
  import { href } from '../lib/router';
  import GargoyleMark from './GargoyleMark.svelte';
  import HealthBadge from './HealthBadge.svelte';

  let {
    host,
    health,
    receivedAt,
    now,
    stale,
  }: {
    host: string | null;
    health: Health | null;
    /** Client time of the last successful update; null before the first. */
    receivedAt: number | null;
    /** Client clock, ticking. */
    now: number;
    stale: boolean;
  } = $props();
</script>

<header class="header">
  <div class="inner">
    <a class="brand" href={href()} aria-label="Gargoyle overview">
      <GargoyleMark />
      <span class="wordmark">Gargoyle</span>
    </a>
    {#if host}
      <span class="host" title="Host">{host}</span>
    {/if}
    <div class="status">
      {#if receivedAt !== null}
        <span class="updated" class:stale title="Last updated {formatTimestamp(receivedAt)}">
          Updated <span class="ago">{formatRelativeTime(receivedAt, now)}</span>
        </span>
      {/if}
      {#if health}<HealthBadge {health} size="sm" />{/if}
    </div>
  </div>
</header>

<style>
  .header {
    position: sticky;
    top: 0;
    z-index: 1;
    border-bottom: 1px solid var(--color-border);
    background: var(--color-surface);
  }

  .inner {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1) var(--space-3);
    max-width: var(--content-max-width);
    min-height: var(--header-height);
    margin: 0 auto;
    padding: var(--space-2) var(--space-4);
  }

  .brand {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    color: inherit;
    text-decoration: none;
  }

  .brand:hover {
    color: inherit;
  }

  .wordmark {
    font-size: var(--text-lg);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-wide);
  }

  .host {
    min-width: 0;
    padding-left: var(--space-3);
    border-left: 1px solid var(--color-border);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }

  .status {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-left: auto;
  }

  .updated {
    font-size: var(--text-xs);
    color: var(--color-text-faint);
    white-space: nowrap;
  }

  .updated.stale {
    color: var(--health-warning-fg);
  }

  .ago {
    font-variant-numeric: tabular-nums;
  }
</style>
