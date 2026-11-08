<script lang="ts">
  import type { TimerSummary } from '../../lib/api/types';
  import { formatCountdown, formatRelativeTime, formatTimestamp } from '../../lib/format';
  import { needsAttention, sortBySeverity } from '../../lib/health';
  import { href } from '../../lib/router';
  import { isRunning, runOutcome } from '../../lib/timers';
  import HealthBadge from '../HealthBadge.svelte';
  import RunOutcome from './RunOutcome.svelte';

  // The overview's timers: a table on wide screens, one card per timer on narrow ones.
  let { timers, serverNow }: { timers: TimerSummary[]; serverNow: number } = $props();

  const sorted = $derived(sortBySeverity(timers));

  // The last-run column already says "failed" or "never run"; repeating the backend's message
  // under the name would say it twice. Other reasons (inactive, overdue, too old) are shown.
  function explainedByLastRun(timer: TimerSummary): boolean {
    if (isRunning(timer)) return false;
    if (timer.lastRun === null) return timer.health === 'unknown';
    return timer.health === 'critical' && runOutcome(timer.lastRun) === 'failed';
  }

  const showMessage = (timer: TimerSummary) =>
    needsAttention(timer.health) && timer.message !== null && !explainedByLastRun(timer);
</script>

<table class="table">
  <thead>
    <tr>
      <th scope="col">Timer</th>
      <th scope="col">Last run</th>
      <th scope="col">Next run</th>
      <th scope="col"><span class="sr-only">Health</span></th>
    </tr>
  </thead>
  <tbody>
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
            <span class="countdown" title={formatTimestamp(timer.nextRun)}>
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
  </tbody>
</table>

<style>
  .table {
    width: 100%;
    border-collapse: separate;
    border-spacing: 0;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-sm);
    font-size: var(--text-sm);
    overflow: hidden;
  }

  th,
  td {
    padding: var(--space-3) var(--space-4);
    text-align: left;
    vertical-align: top;
    border-bottom: 1px solid var(--color-border);
  }

  tbody tr:last-child > * {
    border-bottom: none;
  }

  thead th {
    padding-block: var(--space-2);
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
    text-transform: uppercase;
    letter-spacing: var(--tracking-wide);
    background: var(--color-surface-sunken);
    color: var(--color-text-muted);
  }

  td {
    font-variant-numeric: tabular-nums;
    color: var(--color-text-muted);
  }

  td > span {
    display: block;
  }

  .name-cell {
    font-weight: var(--weight-regular);
    min-width: 0;
  }

  [data-health='warning'] > .name-cell {
    box-shadow: inset 3px 0 0 var(--health-warning-fg);
  }
  [data-health='critical'] > .name-cell {
    box-shadow: inset 3px 0 0 var(--health-critical-fg);
  }
  [data-health='unknown'] > .name-cell {
    box-shadow: inset 3px 0 0 var(--health-unknown-fg);
  }

  .name {
    display: block;
    font-size: var(--text-md);
    font-weight: var(--weight-medium);
    color: inherit;
    text-decoration-color: var(--color-border-strong);
    text-underline-offset: 0.2em;
    overflow-wrap: anywhere;
  }

  .name:hover {
    color: var(--color-accent-hover);
    text-decoration-color: currentColor;
  }

  .id {
    display: block;
    font-size: var(--text-xs);
    color: var(--color-text-faint);
    overflow-wrap: anywhere;
  }

  .message {
    margin-top: var(--space-1);
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

  .countdown,
  td > span:first-child {
    color: var(--color-text);
  }

  .sub {
    font-size: var(--text-xs);
  }

  .never {
    font-style: italic;
  }

  .running {
    font-weight: var(--weight-medium);
  }

  .health-cell {
    text-align: right;
  }

  /* Narrow screens: one card per timer, name and health on top, timings labelled below. */
  @media (max-width: 40rem) {
    /* Without this the grid rows shrink to fit inside the table box. */
    .table,
    tbody {
      display: block;
    }

    thead {
      position: absolute;
      width: 1px;
      height: 1px;
      overflow: hidden;
      clip-path: inset(50%);
    }

    /* Name and badge on the first line, the two timings side by side below. */
    tbody tr {
      display: flex;
      flex-wrap: wrap;
      align-items: flex-start;
      border-bottom: 1px solid var(--color-border);
    }

    tbody tr:last-child {
      border-bottom: none;
    }

    th,
    td {
      border-bottom: none;
    }

    /* Wide enough that a timing can never join the first line. */
    .name-cell {
      flex: 1 1 60%;
      order: 0;
      padding-bottom: var(--space-1);
    }

    .health-cell {
      flex: none;
      order: 1;
      padding-bottom: var(--space-1);
    }

    td[data-label] {
      flex: 1 1 40%;
      order: 2;
      padding-top: 0;
    }

    td[data-label]::before {
      content: attr(data-label);
      display: block;
      font-size: var(--text-xs);
      color: var(--color-text-faint);
    }

    [data-health] > .name-cell {
      box-shadow: none;
    }

    tbody tr[data-health='warning'] {
      box-shadow: inset 3px 0 0 var(--health-warning-fg);
    }
    tbody tr[data-health='critical'] {
      box-shadow: inset 3px 0 0 var(--health-critical-fg);
    }
    tbody tr[data-health='unknown'] {
      box-shadow: inset 3px 0 0 var(--health-unknown-fg);
    }
  }
</style>
