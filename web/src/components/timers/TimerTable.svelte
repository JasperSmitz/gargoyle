<script lang="ts">
  import type { TimerSummary } from '../../lib/api/types';
  import { formatCountdown, formatRelativeTime, formatTimestamp } from '../../lib/format';
  import { needsAttention, sortBySeverity } from '../../lib/health';
  import { href } from '../../lib/router';
  import { isRunning, runOutcome } from '../../lib/timers';
  import DataTable from '../DataTable.svelte';
  import HealthBadge from '../HealthBadge.svelte';
  import RunOutcome from './RunOutcome.svelte';

  // The overview's timers; layout and row conventions come from DataTable.
  let { timers, serverNow }: { timers: TimerSummary[]; serverNow: number } = $props();

  const sorted = $derived(sortBySeverity(timers));

  // Every non-healthy row shows the backend's message (M4 acceptance), even where the last-run
  // column already hints at it: the message is the one place that says why.
  const showMessage = (timer: TimerSummary) =>
    needsAttention(timer.health) && timer.message !== null;
</script>

<DataTable>
  {#snippet head()}
    <th scope="col">Timer</th>
    <th scope="col">Last run</th>
    <th scope="col">Next run</th>
    <th scope="col"><span class="sr-only">Health</span></th>
  {/snippet}
  {#each sorted as timer (timer.unit)}
    <tr data-health={timer.health}>
      <th scope="row" class="name-cell">
        <a class="name" href={href('timers', timer.unit)}>{timer.name}</a>
        {#if timer.unit !== timer.name}<code class="id">{timer.unit}</code>{/if}
        {#if showMessage(timer)}<p class="message">{timer.message}</p>{/if}
      </th>
      <td data-label="Last run">
        {#if isRunning(timer)}
          <span class="running">Running now</span>
          <!-- lastRun may still be the previous, finished run if the flag got there first. -->
          {#if timer.lastRun && runOutcome(timer.lastRun) === 'running'}
            <span class="sub" title={formatTimestamp(timer.lastRun.startedAt)}>
              started {formatRelativeTime(timer.lastRun.startedAt, serverNow)}
            </span>
          {/if}
        {:else if timer.lastRun}
          <span title={formatTimestamp(timer.lastRun.startedAt)}>
            {formatRelativeTime(timer.lastRun.startedAt, serverNow)}
          </span>
          <span class="sub"><RunOutcome run={timer.lastRun} /></span>
        {:else}
          <span class="never">Never run</span>
        {/if}
      </td>
      <td data-label="Next run">
        {#if timer.nextRun !== null}
          <span title={formatTimestamp(timer.nextRun)}>
            {formatCountdown(timer.nextRun, serverNow)}
          </span>
        {:else}
          <span class="never">Not scheduled</span>
        {/if}
        {#if !timer.active}<span class="sub">timer inactive</span>{/if}
      </td>
      <td class="health-cell"><HealthBadge health={timer.health} size="sm" /></td>
    </tr>
  {/each}
</DataTable>

<style>
  .running {
    font-weight: var(--weight-medium);
  }
</style>
