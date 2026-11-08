<script lang="ts">
  import type { TimerRunSummary, TimerSummary } from '../../lib/api/types';
  import { formatDuration, formatRelativeTime, formatTimestamp } from '../../lib/format';
  import { needsAttention, sortBySeverity } from '../../lib/health';
  import CardList from '../CardList.svelte';
  import Section from '../Section.svelte';
  import TargetRow from '../TargetRow.svelte';

  // Simple list for now; the timers table with countdowns and detail pages arrive in M3.6.
  let { timers, serverNow }: { timers: TimerSummary[]; serverNow: number } = $props();

  const sorted = $derived(sortBySeverity(timers));

  // A non-healthy row already shows the backend's message ("last run failed: …", "never run"),
  // so the meta line keeps only the timing to avoid saying it twice.
  const explained = (timer: TimerSummary) => needsAttention(timer.health) && timer.message !== null;

  function outcome(run: TimerRunSummary): string {
    if (run.result === null) return 'in progress';
    const status =
      run.exitStatus !== null && run.exitStatus !== 0 ? ` (status ${run.exitStatus})` : '';
    const took = run.duration !== null ? `, took ${formatDuration(run.duration)}` : '';
    return (run.result === 'success' ? 'succeeded' : run.result + status) + took;
  }
</script>

<Section title="Timers" count={timers.length}>
  <CardList>
    {#each sorted as timer (timer.unit)}
      <TargetRow name={timer.name} id={timer.unit} health={timer.health} message={timer.message}>
        {#snippet meta()}
          {#if timer.running}
            <span>running now</span>
          {:else if timer.lastRun}
            <span title={formatTimestamp(timer.lastRun.startedAt)}>
              last {formatRelativeTime(timer.lastRun.startedAt, serverNow)}{#if !explained(timer)}
                · {outcome(timer.lastRun)}{/if}
            </span>
          {:else if !explained(timer)}
            <span>never run</span>
          {/if}
          {#if timer.nextRun !== null}
            <span title={formatTimestamp(timer.nextRun)}>
              next {formatRelativeTime(timer.nextRun, serverNow)}
            </span>
          {/if}
          {#if !timer.active}<span>timer inactive</span>{/if}
        {/snippet}
      </TargetRow>
    {/each}
  </CardList>
</Section>
