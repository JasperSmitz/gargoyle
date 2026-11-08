<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { Health } from '../../lib/api/types';
  import { breakPoints } from '../../lib/format';
  import { needsAttention } from '../../lib/health';
  import HealthBadge from '../HealthBadge.svelte';
  import SkeletonBlock from '../skeleton/SkeletonBlock.svelte';

  // Top of a detail page: name, unit, health with its reason, then key facts as a <dl>. It stays
  // mounted from the first render (name from the URL) through loading, errors and data, so the
  // h1 that route focus lands on is never replaced under the user.
  let {
    kind,
    name,
    unit,
    health,
    message,
    facts,
    loading = null,
  }: {
    /** "Timer", "Service". */
    kind: string;
    name: string;
    unit: string;
    /** Null until the detail has loaded (or when it could not be). */
    health: Health | null;
    message: string | null;
    /** `<div><dt/><dd/></div>` groups; only rendered with a health. */
    facts?: Snippet;
    /** Status label while the first response is pending, e.g. "Loading timer". */
    loading?: string | null;
  } = $props();
</script>

<section class="header health-{health ?? 'none'}" aria-label="{kind} summary">
  <div class="title">
    <div class="names">
      <span class="kind">{kind}</span>
      <h1>{name}</h1>
      {#if unit !== name}
        <!-- <wbr> lets a long URL wrap between segments; overflow-wrap is the fallback. -->
        <code class="unit"
          >{#each breakPoints(unit) as part, i (i)}{#if i > 0}<wbr />{/if}{part}{/each}</code
        >
      {/if}
    </div>
    {#if health}<HealthBadge {health} />{/if}
  </div>
  {#if health && message && needsAttention(health)}<p class="message">{message}</p>{/if}
  {#if loading}
    <div class="facts placeholder" role="status" aria-label={loading}>
      {#each { length: 4 }, i (i)}
        <div>
          <SkeletonBlock height="0.75rem" width="4rem" radius="var(--radius-sm)" />
          <SkeletonBlock height="1.25rem" width="7rem" radius="var(--radius-sm)" />
        </div>
      {/each}
    </div>
  {:else if health && facts}
    <dl class="facts">{@render facts()}</dl>
  {/if}
</section>

<style>
  .header {
    display: grid;
    gap: var(--space-3);
    padding: var(--space-4);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-left-width: 4px;
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-sm);
  }

  .health-none {
    border-left-color: var(--color-border-strong);
  }
  .health-healthy {
    border-left-color: var(--health-healthy-fg);
  }
  .health-warning {
    border-left-color: var(--health-warning-fg);
  }
  .health-critical {
    border-left-color: var(--health-critical-fg);
  }
  .health-unknown {
    border-left-color: var(--health-unknown-fg);
  }

  .title {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-2) var(--space-4);
  }

  .names {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0 var(--space-3);
    min-width: 0;
  }

  .kind {
    flex-basis: 100%;
    font-size: var(--text-xs);
    color: var(--color-text-faint);
    text-transform: uppercase;
    letter-spacing: var(--tracking-wide);
  }

  h1 {
    font-size: var(--text-xl);
    overflow-wrap: anywhere;
  }

  .unit {
    font-size: var(--text-sm);
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }

  .message {
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }
  .health-warning .message {
    color: var(--health-warning-fg);
  }
  .health-critical .message {
    color: var(--health-critical-fg);
  }
  .health-unknown .message {
    color: var(--health-unknown-fg);
  }

  .facts {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 11rem), 1fr));
    gap: var(--space-3) var(--space-4);
    margin: 0;
    padding-top: var(--space-3);
    border-top: 1px solid var(--color-border);
  }

  .placeholder > div {
    display: grid;
    gap: var(--space-1);
  }

  .facts :global(dt) {
    font-size: var(--text-xs);
    color: var(--color-text-faint);
  }

  .facts :global(dd) {
    margin: 0;
    font-variant-numeric: tabular-nums;
    overflow-wrap: anywhere;
  }

  .facts :global(.sub) {
    display: block;
    font-size: var(--text-xs);
    color: var(--color-text-muted);
  }
</style>
