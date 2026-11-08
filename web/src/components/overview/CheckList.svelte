<script lang="ts">
  import type { CheckSummary } from '../../lib/api/types';
  import { formatRelativeTime, formatTimestamp } from '../../lib/format';
  import { sortBySeverity } from '../../lib/health';
  import CardList from '../CardList.svelte';
  import Section from '../Section.svelte';
  import TargetRow from '../TargetRow.svelte';

  // Simple list for now; check detail pages with latency charts arrive in M4.4.
  let { checks, serverNow }: { checks: CheckSummary[]; serverNow: number } = $props();

  const sorted = $derived(sortBySeverity(checks));
</script>

<Section title="Checks" count={checks.length}>
  <CardList>
    {#each sorted as check (check.name)}
      <TargetRow name={check.name} id={check.target} health={check.health} message={check.message}>
        {#snippet meta()}
          <span class="kind">{check.kind}</span>
          {#if check.latencyMs !== null}<span>{check.latencyMs.toFixed(1)} ms</span>{/if}
          {#if check.lastCheck !== null}
            <span title={formatTimestamp(check.lastCheck)}>
              checked {formatRelativeTime(check.lastCheck, serverNow)}
            </span>
          {:else}
            <span>not run yet</span>
          {/if}
          {#if check.consecutiveFailures > 0}
            <span>
              {check.consecutiveFailures}
              {check.consecutiveFailures === 1 ? 'failure' : 'failures'} in a row
            </span>
          {/if}
        {/snippet}
      </TargetRow>
    {/each}
  </CardList>
</Section>

<style>
  .kind {
    text-transform: uppercase;
    letter-spacing: var(--tracking-wide);
    font-size: var(--text-xs);
    align-self: center;
  }
</style>
