<script lang="ts">
  import type { HealthEvent } from '../../lib/api/types';
  import { formatRelativeTime } from '../../lib/format';
  import Time from '../Time.svelte';
  import { targetHref } from '../../lib/router';
  import HealthLabel from '../HealthLabel.svelte';

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
  <span class="when"><Time ts={event.ts} text={formatRelativeTime(event.ts, serverNow)} /></span>
  <div class="body">
    <div class="line">
      {#if showTarget}
        <a class="target" href={link}><code>{event.target}</code></a>
      {/if}
      <span class="transition">
        {#if event.from}
          <HealthLabel health={event.from} />
          <span aria-label="to">→</span>
        {/if}
        <HealthLabel health={event.to} />
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

  .when {
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

  .message {
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }
</style>
