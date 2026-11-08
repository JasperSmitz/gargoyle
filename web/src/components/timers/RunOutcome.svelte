<script lang="ts">
  import type { TimerRunSummary } from '../../lib/api/types';
  import { OUTCOME_HEALTH, outcomeLabel, runOutcome } from '../../lib/timers';
  import HealthIcon from '../HealthIcon.svelte';

  // A run's result as icon plus text, colored by outcome.
  let { run }: { run: TimerRunSummary } = $props();

  const outcome = $derived(runOutcome(run));
</script>

<span class="outcome {outcome}" data-outcome={outcome}>
  <HealthIcon health={OUTCOME_HEALTH[outcome]} size={12} />{outcomeLabel(run)}
</span>

<style>
  .outcome {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    font-weight: var(--weight-medium);
    overflow-wrap: anywhere;
  }

  .success {
    color: var(--health-healthy-fg);
  }
  .failed {
    color: var(--health-critical-fg);
  }
  .running {
    color: var(--color-accent);
  }
</style>
