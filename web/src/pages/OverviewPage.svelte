<script lang="ts">
  import type { Overview } from '../lib/api/types';
  import type { Column } from '../lib/charts/data';
  import CheckTable from '../components/checks/CheckTable.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import ConfigHint from '../components/overview/ConfigHint.svelte';
  import EventList from '../components/overview/EventList.svelte';
  import LibraryCard from '../components/overview/LibraryCard.svelte';
  import ServiceList from '../components/overview/ServiceList.svelte';
  import StatusSummary from '../components/overview/StatusSummary.svelte';
  import SystemCards from '../components/overview/SystemCards.svelte';
  import Section from '../components/Section.svelte';
  import TimerTable from '../components/timers/TimerTable.svelte';

  // Order is the hierarchy: verdict and problems, then the system, then each kind of target,
  // then what changed recently. Kinds with nothing configured are left out and named once by
  // ConfigHint instead of each getting an empty box.
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

  const KINDS = ['service', 'timer', 'check'] as const;
  const configured = $derived({
    service: overview.services.length,
    timer: overview.timers.length,
    check: overview.checks.length,
  });
  const missing = $derived(KINDS.filter((kind) => configured[kind] === 0));
  // The library is optional too: when it is watched, "only system metrics" would be untrue, so the
  // hint counts it as one more kind and falls back to its one-line form.
  const kinds = $derived(KINDS.length + (overview.library ? 1 : 0));
</script>

<div class="page">
  <StatusSummary {overview} />

  <Section title="System">
    <SystemCards system={overview.system} hostInfo={overview.hostInfo} {serverNow} {history} />
  </Section>

  {#if missing.length === kinds}
    <ConfigHint {missing} total={kinds} healthy={overview.health === 'healthy'} />
  {/if}

  {#if overview.services.length > 0}
    <ServiceList services={overview.services} {serverNow} />
  {/if}

  <!-- Full width: the tables' timing columns need the room. -->
  {#if overview.checks.length > 0}
    <Section title="Checks" count={overview.checks.length}>
      <CheckTable checks={overview.checks} {serverNow} />
    </Section>
  {/if}

  {#if overview.timers.length > 0}
    <Section title="Timers" count={overview.timers.length}>
      <TimerTable timers={overview.timers} {serverNow} />
    </Section>
  {/if}

  {#if overview.library}
    <LibraryCard library={overview.library} {serverNow} />
  {/if}

  {#if missing.length > 0 && missing.length < kinds}
    <ConfigHint {missing} total={kinds} healthy={overview.health === 'healthy'} />
  {/if}

  {#if overview.recentEvents.length > 0}
    <EventList events={overview.recentEvents} {serverNow} />
  {:else}
    <Section title="Recent events">
      <EmptyState title="No health changes recorded yet">
        Every time a target changes health it is logged here.
      </EmptyState>
    </Section>
  {/if}
</div>

<style>
  .page {
    display: grid;
    gap: var(--space-6);
  }
</style>
