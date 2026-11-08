<script lang="ts">
  import type { CheckSummary } from '../../lib/api/types';
  import { CHECK_KIND_LABEL, formatLatency } from '../../lib/checks';
  import { breakPoints, formatRelativeTime, formatTimestamp } from '../../lib/format';
  import { needsAttention, sortBySeverity } from '../../lib/health';
  import { href } from '../../lib/router';
  import DataTable from '../DataTable.svelte';
  import HealthBadge from '../HealthBadge.svelte';

  // The overview's HTTP/TCP checks, laid out like the timers table. `target` arrives sanitized
  // (no userinfo or query, ARCHITECTURE §6) and is shown as-is.
  let { checks, serverNow }: { checks: CheckSummary[]; serverNow: number } = $props();

  const sorted = $derived(sortBySeverity(checks));
</script>

<DataTable>
  {#snippet head()}
    <th scope="col">Check</th>
    <th scope="col">Latency</th>
    <th scope="col">Last check</th>
    <th scope="col">Failures</th>
    <th scope="col"><span class="sr-only">Health</span></th>
  {/snippet}
  {#each sorted as check (check.name)}
    {@const latency = formatLatency(check)}
    <tr data-health={check.health}>
      <th scope="row" class="name-cell">
        <a class="name" href={href('checks', check.name)}>{check.name}</a>
        <span class="id">
          <span class="kind">{CHECK_KIND_LABEL[check.kind]}</span>
          <code
            >{#each breakPoints(check.target) as part, i (i)}{#if i > 0}<wbr
                />{/if}{part}{/each}</code
          >
        </span>
        {#if needsAttention(check.health) && check.message}
          <p class="message">{check.message}</p>
        {/if}
      </th>
      <td data-label="Latency">
        {#if latency !== null}
          <span>{latency}</span>
        {:else}
          <span class="never">{check.lastCheck === null ? 'Not run yet' : 'Failed'}</span>
        {/if}
      </td>
      <td data-label="Last check">
        {#if check.lastCheck !== null}
          <span title={formatTimestamp(check.lastCheck)}>
            {formatRelativeTime(check.lastCheck, serverNow)}
          </span>
        {:else}
          <span class="never">Never</span>
        {/if}
      </td>
      <td data-label="Failures">
        <span>{check.consecutiveFailures}</span>
        {#if check.consecutiveFailures > 0}<span class="sub">in a row</span>{/if}
      </td>
      <td class="health-cell"><HealthBadge health={check.health} size="sm" /></td>
    </tr>
  {/each}
</DataTable>

<style>
  .kind {
    margin-right: var(--space-1);
    font-weight: var(--weight-medium);
    letter-spacing: var(--tracking-wide);
  }
</style>
