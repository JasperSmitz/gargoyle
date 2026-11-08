<script lang="ts">
  import type { Overview } from '../../lib/api/types';
  import { HEALTH_BY_SEVERITY } from '../../lib/health';
  import { collectProblems } from '../../lib/problems';
  import HealthBadge from '../HealthBadge.svelte';
  import HealthIcon from '../HealthIcon.svelte';

  // First thing on the page: the overall verdict, then exactly what is wrong.
  let { overview }: { overview: Overview } = $props();

  const headingId = $props.id();
  const problems = $derived(collectProblems(overview));
  // The headline uses the backend's target counts (the system is one target, §8); the list below
  // may name several system metrics individually.
  const total = $derived(Object.values(overview.counts).reduce((a, b) => a + b, 0));
  const unhealthy = $derived(total - overview.counts.healthy);
  const headline = $derived(
    unhealthy === 0
      ? `All ${total} ${total === 1 ? 'target' : 'targets'} healthy`
      : `${unhealthy} of ${total} ${total === 1 ? 'target needs' : 'targets need'} attention`,
  );
  const KIND_LABEL = { system: 'System', service: 'Service', timer: 'Timer', check: 'Check' };
</script>

<section class="summary health-{overview.health}" aria-labelledby={headingId}>
  <div class="verdict">
    <HealthBadge health={overview.health} size="lg" />
    <h1 class="headline" id={headingId}>{headline}</h1>
  </div>
  <ul class="counts" aria-label="Targets by health">
    {#each HEALTH_BY_SEVERITY as health (health)}
      {#if overview.counts[health] > 0}
        <li><HealthBadge {health} size="sm" label="{overview.counts[health]} {health}" /></li>
      {/if}
    {/each}
  </ul>

  {#if problems.length > 0}
    <ul class="problems" aria-label="Needs attention">
      {#each problems as problem (problem.id)}
        <li class="problem health-{problem.health}">
          <HealthIcon health={problem.health} />
          <span class="what">
            <span class="name">{problem.name}</span>
            <span class="kind">{KIND_LABEL[problem.kind]}</span>
          </span>
          <span class="message">{problem.message}</span>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .summary {
    display: grid;
    gap: var(--space-3);
    padding: var(--space-4);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-left-width: 4px;
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-sm);
  }

  .summary.health-healthy {
    border-left-color: var(--health-healthy-fg);
  }
  .summary.health-warning {
    border-left-color: var(--health-warning-fg);
  }
  .summary.health-critical {
    border-left-color: var(--health-critical-fg);
  }
  .summary.health-unknown {
    border-left-color: var(--health-unknown-fg);
  }

  .verdict {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
  }

  .headline {
    font-size: var(--text-xl);
  }

  .counts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .problems {
    display: grid;
    margin: 0;
    padding: 0;
    list-style: none;
    border-top: 1px solid var(--color-border);
  }

  .problem {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 0 var(--space-2);
    align-items: baseline;
    padding-block: var(--space-2);
    border-bottom: 1px solid var(--color-border);
  }

  .problem:last-child {
    border-bottom: none;
    padding-bottom: 0;
  }

  .problem :global(.icon) {
    align-self: center;
  }

  .what {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0 var(--space-2);
    min-width: 0;
  }

  .name {
    font-weight: var(--weight-medium);
    overflow-wrap: anywhere;
  }

  .kind {
    font-size: var(--text-xs);
    color: var(--color-text-faint);
    text-transform: uppercase;
    letter-spacing: var(--tracking-wide);
  }

  .message {
    grid-column: 2;
    font-size: var(--text-sm);
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }

  .health-warning > :global(.icon) {
    color: var(--health-warning-fg);
  }
  .health-critical > :global(.icon) {
    color: var(--health-critical-fg);
  }
  .health-unknown > :global(.icon) {
    color: var(--health-unknown-fg);
  }

  @media (min-width: 40rem) {
    .problem {
      grid-template-columns: auto minmax(10rem, 16rem) 1fr;
    }

    .message {
      grid-column: 3;
    }
  }
</style>
