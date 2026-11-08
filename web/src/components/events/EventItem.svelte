<script lang="ts">
  import type { HealthEvent } from '../../lib/api/types';
  import { formatRelativeTime, formatTimestamp } from '../../lib/format';
  import { HEALTH_PRESENTATION } from '../../lib/health';
  import { targetHref } from '../../lib/router';
  import HealthIcon from '../HealthIcon.svelte';

  // One health transition, as a row of a CardList.
  let {
    event,
    serverNow,
    showTarget = true,
  }: {
    event: HealthEvent;
    serverNow: number;
    /** Off on a target's own detail page, where the target is already the page's subject. */
    showTarget?: boolean;
  } = $props();

  const link = $derived(targetHref(event.targetKind, event.target));
</script>

<li class="event">
  <time datetime={new Date(event.ts * 1000).toISOString()} title={formatTimestamp(event.ts)}>
    {formatRelativeTime(event.ts, serverNow)}
  </time>
  <div class="body">
    <div class="line">
      {#if showTarget}
        {#if link}
          <a class="target" href={link}><code>{event.target}</code></a>
        {:else}
          <code class="target">{event.target}</code>
        {/if}
      {/if}
      <span class="transition">
        {#if event.from}
          <span class="state health-{event.from}">
            <HealthIcon health={event.from} size={12} />{HEALTH_PRESENTATION[event.from].label}
          </span>
          <span aria-label="to">→</span>
        {/if}
        <span class="state health-{event.to}">
          <HealthIcon health={event.to} size={12} />{HEALTH_PRESENTATION[event.to].label}
        </span>
      </span>
    </div>
    {#if event.message}<p class="message">{event.message}</p>{/if}
  </div>
</li>

<style>
  .event {
    display: grid;
    grid-template-columns: 4.5rem 1fr;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--color-border);
    font-size: var(--text-sm);
  }

  .event:last-child {
    border-bottom: none;
  }

  time {
    color: var(--color-text-faint);
    font-variant-numeric: tabular-nums;
  }

  .body {
    display: grid;
    gap: var(--space-1);
    min-width: 0;
  }

  .line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1) var(--space-3);
  }

  .target {
    overflow-wrap: anywhere;
  }

  .transition {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--color-text-faint);
  }

  .state {
    display: inline-flex;
    align-items: center;
    gap: 0.2rem;
    font-weight: var(--weight-medium);
  }

  .health-healthy {
    color: var(--health-healthy-fg);
  }
  .health-warning {
    color: var(--health-warning-fg);
  }
  .health-critical {
    color: var(--health-critical-fg);
  }
  .health-unknown {
    color: var(--health-unknown-fg);
  }

  .message {
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }
</style>
