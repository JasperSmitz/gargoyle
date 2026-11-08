<script lang="ts">
  import type { TimerRun } from '../../lib/api/types';
  import { formatDuration, formatRelativeTime, formatTimestamp } from '../../lib/format';
  import { durationFractions, runDuration, runOutcome } from '../../lib/timers';
  import RunOutcome from './RunOutcome.svelte';

  // Newest first. Each run's duration is a bar scaled to the longest run shown, so slow or
  // cut-short runs stand out; the exact duration sits at the bar's tip and the result beside it.
  let { runs, serverNow }: { runs: TimerRun[]; serverNow: number } = $props();

  const durations = $derived(runs.map((run) => runDuration(run, serverNow)));
  const fractions = $derived(durationFractions(durations));
  const longest = $derived(Math.max(0, ...durations.map((d) => d ?? 0)));
  const id = $props.id();

  function tooltip(run: TimerRun): string {
    const start = `Started ${formatTimestamp(run.startedAt)}`;
    const end = run.finishedAt !== null ? `, finished ${formatTimestamp(run.finishedAt)}` : '';
    const source = run.source === 'journal' ? ' (from the journal)' : '';
    return start + end + source;
  }
</script>

<p class="scale" id="{id}-scale">
  Bars compare run durations; the longest shown took {formatDuration(longest)}.
</p>
<ol class="runs" aria-label="Run history" aria-describedby="{id}-scale">
  <!-- Index in the key: a journal and an observed row may share a start second until M3.3/M3.4 dedupe. -->
  {#each runs as run, i (`${run.startedAt}-${i}`)}
    {@const outcome = runOutcome(run)}
    {@const fraction = fractions[i] ?? null}
    <li class="run" title={tooltip(run)}>
      <span class="when">
        <time datetime={new Date(run.startedAt * 1000).toISOString()}>
          {formatRelativeTime(run.startedAt, serverNow)}
        </time>
        <span class="absolute">{formatTimestamp(run.startedAt)}</span>
      </span>
      <span class="result">
        <RunOutcome {run} />
        {#if run.source === 'journal'}<span class="source">journal</span>{/if}
      </span>
      <span class="duration">
        {#if fraction !== null}
          <span class="fill {outcome}" style:--fraction={fraction}></span>
        {/if}
        <span class="value">
          {#if outcome === 'running'}{formatDuration(durations[i] ?? null)} so far{:else}{formatDuration(
              durations[i] ?? null,
            )}{/if}
        </span>
      </span>
    </li>
  {/each}
</ol>

<style>
  .scale {
    font-size: var(--text-xs);
    color: var(--color-text-faint);
  }

  .runs {
    margin: 0;
    padding: 0;
    list-style: none;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-sm);
    overflow: hidden;
  }

  .run {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    grid-template-areas:
      'when result'
      'duration duration';
    gap: var(--space-1) var(--space-3);
    align-items: center;
    padding: var(--space-2) var(--space-4);
    border-bottom: 1px solid var(--color-border);
    font-size: var(--text-sm);
  }

  .run:last-child {
    border-bottom: none;
  }

  .when {
    grid-area: when;
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0 var(--space-2);
    min-width: 0;
    font-variant-numeric: tabular-nums;
  }

  .absolute {
    font-size: var(--text-xs);
    color: var(--color-text-faint);
  }

  .result {
    grid-area: result;
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
  }

  .source {
    font-size: var(--text-xs);
    color: var(--color-text-faint);
  }

  /* The label is the bar's tip: the fill takes its share of the space left beside the label. */
  .duration {
    --label-space: 6.5rem;
    grid-area: duration;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .fill {
    flex: none;
    width: calc(var(--fraction) * (100% - var(--label-space)));
    height: 8px;
    border-radius: 0 4px 4px 0;
  }

  /* Success is the normal case and stays quiet; failures are what the eye should find. */
  .fill.success {
    background: var(--chart-spark);
  }
  .fill.failed {
    background: var(--health-critical-fg);
  }
  .fill.running {
    background: repeating-linear-gradient(
      135deg,
      var(--color-accent) 0 4px,
      color-mix(in srgb, var(--color-accent) 45%, transparent) 4px 8px
    );
  }

  .value {
    flex: none;
    color: var(--color-text-muted);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  @media (min-width: 48rem) {
    .run {
      grid-template-columns: 15rem 12rem minmax(0, 1fr);
      grid-template-areas: 'when result duration';
    }

    .result {
      justify-content: flex-start;
    }
  }
</style>
