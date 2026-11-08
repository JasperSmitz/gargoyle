<script lang="ts">
  import type { Overview } from '../lib/api/types';
  import type { Column } from '../lib/charts/data';
  import CheckList from '../components/overview/CheckList.svelte';
  import EventList from '../components/overview/EventList.svelte';
  import ServiceList from '../components/overview/ServiceList.svelte';
  import StatusSummary from '../components/overview/StatusSummary.svelte';
  import SystemCards from '../components/overview/SystemCards.svelte';
  import Section from '../components/Section.svelte';
  import TimerTable from '../components/timers/TimerTable.svelte';

  // Order is the hierarchy: verdict and problems, then the system, then each kind of target.
  let {
    overview,
    serverNow,
    history = null,
  }: {
    overview: Overview;
    serverNow: number;
    /** Last-hour averages by series key for sparklines; null until loaded (or unavailable). */
    history?: Map<string, Column> | null;
  } = $props();
</script>

<div class="page">
  <StatusSummary {overview} />

  <Section title="System">
    <SystemCards system={overview.system} hostInfo={overview.hostInfo} {serverNow} {history} />
  </Section>

  <div class="columns">
    <ServiceList services={overview.services} {serverNow} />
    {#if overview.checks.length > 0}<CheckList checks={overview.checks} {serverNow} />{/if}
  </div>

  <!-- Full width: four columns of timings need the room. -->
  {#if overview.timers.length > 0}
    <Section title="Timers" count={overview.timers.length}>
      <TimerTable timers={overview.timers} {serverNow} />
    </Section>
  {/if}

  {#if overview.recentEvents.length > 0}
    <EventList events={overview.recentEvents} {serverNow} />
  {/if}
</div>

<style>
  .page {
    display: grid;
    gap: var(--space-6);
  }

  .columns {
    display: grid;
    gap: var(--space-6);
    align-items: start;
  }

  @media (min-width: 64rem) {
    .columns {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
</style>
