<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { Health } from '../lib/api/types';
  import { needsAttention } from '../lib/health';
  import HealthBadge from './HealthBadge.svelte';

  // One service, timer or check in a list. Non-healthy rows always show their message (M4 rule:
  // every non-healthy state explains itself).
  let {
    name,
    id,
    health,
    message,
    href,
    meta,
  }: {
    name: string;
    /** Technical identifier shown under the name, e.g. the unit; omitted when equal to name. */
    id?: string;
    health: Health;
    message: string | null;
    /** Detail page; the name becomes a link to it. */
    href?: string;
    /** Secondary facts (state, last run, latency, ...). */
    meta?: Snippet;
  } = $props();
</script>

<li class="row" data-health={health}>
  <div class="main">
    <div class="names">
      {#if href}<a class="name" {href}>{name}</a>{:else}<span class="name">{name}</span>{/if}
      {#if id && id !== name}<code class="id">{id}</code>{/if}
    </div>
    {#if meta}<div class="meta">{@render meta()}</div>{/if}
    {#if message && needsAttention(health)}<p class="message">{message}</p>{/if}
  </div>
  <HealthBadge {health} size="sm" />
</li>

<style>
  .row {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--color-border);
  }

  .row:last-child {
    border-bottom: none;
  }

  .row[data-health='warning'] {
    box-shadow: inset 3px 0 0 var(--health-warning-fg);
  }
  .row[data-health='critical'] {
    box-shadow: inset 3px 0 0 var(--health-critical-fg);
  }
  .row[data-health='unknown'] {
    box-shadow: inset 3px 0 0 var(--health-unknown-fg);
  }

  .main {
    display: grid;
    gap: var(--space-1);
    min-width: 0;
  }

  .names {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0 var(--space-2);
    min-width: 0;
  }

  .name {
    font-weight: var(--weight-medium);
    overflow-wrap: anywhere;
  }

  a.name {
    color: inherit;
    text-decoration-color: var(--color-border-strong);
    text-underline-offset: 0.2em;
  }

  a.name:hover {
    color: var(--color-accent-hover);
    text-decoration-color: currentColor;
  }

  .id {
    font-size: var(--text-xs);
    color: var(--color-text-faint);
    overflow-wrap: anywhere;
  }

  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 0 var(--space-3);
    font-size: var(--text-sm);
    color: var(--color-text-muted);
    font-variant-numeric: tabular-nums;
  }

  .message {
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }

  [data-health='warning'] .message {
    color: var(--health-warning-fg);
  }
  [data-health='critical'] .message {
    color: var(--health-critical-fg);
  }
  [data-health='unknown'] .message {
    color: var(--health-unknown-fg);
  }
</style>
