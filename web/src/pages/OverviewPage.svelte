<script lang="ts">
  import type { Overview } from '../lib/api/types';
  import CheckList from '../components/overview/CheckList.svelte';
  import EventList from '../components/overview/EventList.svelte';
  import ServiceList from '../components/overview/ServiceList.svelte';
  import StatusSummary from '../components/overview/StatusSummary.svelte';
  import SystemCards from '../components/overview/SystemCards.svelte';
  import TimerList from '../components/overview/TimerList.svelte';
  import Section from '../components/Section.svelte';

  // Order is the hierarchy: verdict and problems, then the system, then each kind of target.
  let { overview, serverNow }: { overview: Overview; serverNow: number } = $props();
</script>

<div class="page">
  <StatusSummary {overview} />

  <Section title="System">
    <SystemCards system={overview.system} hostInfo={overview.hostInfo} {serverNow} />
  </Section>

  <div class="columns">
    <ServiceList services={overview.services} {serverNow} />
    {#if overview.timers.length > 0}<TimerList timers={overview.timers} {serverNow} />{/if}
    {#if overview.checks.length > 0}<CheckList checks={overview.checks} {serverNow} />{/if}
    {#if overview.recentEvents.length > 0}
      <EventList events={overview.recentEvents} {serverNow} />
    {/if}
  </div>
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
