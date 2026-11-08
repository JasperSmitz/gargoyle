<script lang="ts">
  import { formatRelativeTime } from '../lib/format';
  import Time from './Time.svelte';
  import HealthIcon from './HealthIcon.svelte';

  // Shown when polling fails. The page below keeps the last good data, so say how old it is.
  let {
    title,
    detail,
    receivedAt,
    now,
    onretry,
  }: {
    title: string;
    detail: string;
    /** Client time of the last good data; null if there never was any. */
    receivedAt: number | null;
    now: number;
    onretry: () => void;
  } = $props();
</script>

<div class="banner">
  <HealthIcon health="warning" size={18} />
  <div class="text">
    <!-- Only static text is announced; the ticking age below stays outside the live region. -->
    <div role="alert">
      <p class="title">{title}</p>
      <p class="detail">{detail}. Retrying automatically.</p>
    </div>
    {#if receivedAt !== null}
      <p class="detail age">
        Showing data from <Time ts={receivedAt} text={formatRelativeTime(receivedAt, now)} />.
      </p>
    {/if}
  </div>
  <button class="retry" type="button" onclick={onretry}>Retry now</button>
</div>

<style>
  .banner {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    gap: var(--space-2) var(--space-3);
    padding: var(--space-3) var(--space-4);
    color: var(--health-warning-fg);
    background: var(--health-warning-bg);
    border: 1px solid var(--health-warning-border);
    border-radius: var(--radius-lg);
  }

  .banner > :global(.icon) {
    margin-top: 0.15rem;
  }

  .text {
    flex: 1 1 12rem;
    min-width: 0;
  }

  .title {
    font-weight: var(--weight-semibold);
  }

  .detail {
    font-size: var(--text-sm);
    color: var(--color-text);
    overflow-wrap: anywhere;
  }

  .retry {
    padding: var(--space-1) var(--space-3);
    border: 1px solid var(--health-warning-border);
    border-radius: var(--radius-md);
    background: var(--color-surface);
    color: var(--color-text);
    cursor: pointer;
    transition: background-color var(--duration-fast);
  }

  .retry:hover {
    background: var(--color-surface-raised);
  }
</style>
