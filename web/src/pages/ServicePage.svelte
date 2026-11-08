<script lang="ts">
  import Time from '../components/Time.svelte';
  import { onMount, untrack } from 'svelte';
  import CardList from '../components/CardList.svelte';
  import ChartGroupPanel from '../components/charts/ChartGroupPanel.svelte';
  import RangePicker from '../components/charts/RangePicker.svelte';
  import DetailState from '../components/detail/DetailState.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import EventItem from '../components/events/EventItem.svelte';
  import Section from '../components/Section.svelte';
  import type { ApiClient } from '../lib/api/client';
  import type { ServiceSummary } from '../lib/api/types';
  import type { Range } from '../lib/charts/range';
  import { DETAIL_POLL_MS, PolledFeed } from '../lib/feed.svelte';
  import { formatBytes, formatDuration } from '../lib/format';
  import type { Visibility } from '../lib/poller';
  import { hrefWithQuery } from '../lib/router';
  import { serviceChartGroups, treatUnknownKeyAsEmpty } from '../lib/service-charts';

  // App re-creates the page when `unit` changes, so it is read once.
  let {
    unit,
    range,
    client,
    visibility,
    serverNow,
  }: {
    unit: string;
    range: Range;
    client: Pick<ApiClient, 'service' | 'metricsQuery' | 'metrics'>;
    visibility?: Visibility;
    serverNow: number;
  } = $props();

  const feed = new PolledFeed(
    (signal) =>
      client.service(
        untrack(() => unit),
        { signal },
      ),
    {
      intervalMs: DETAIL_POLL_MS,
      visibility: untrack(() => visibility),
    },
  );
  const chartGroups = serviceChartGroups(untrack(() => unit));
  const chartClient = treatUnknownKeyAsEmpty(untrack(() => client));

  onMount(() => {
    feed.start();
    return () => feed.stop();
  });

  /** "Up for", "Failed for", ...: how long the unit has been in its current state. */
  function sinceLabel(service: ServiceSummary): string {
    if (service.activeState === 'active') return 'Up for';
    const state = service.activeState;
    return `${state.charAt(0).toUpperCase()}${state.slice(1)} for`;
  }
</script>

<div class="page">
  <DetailState
    data={feed.data}
    error={feed.error}
    noun="service"
    kind="Service"
    charts={chartGroups.length}
    name={unit}
    onretry={() => feed.refresh()}
    summary={({ service }) => ({
      kind: 'Service',
      name: service.name,
      unit: service.unit,
      health: service.health,
      message: service.message,
    })}
  >
    {#snippet facts({ service })}
      <div>
        <dt>State</dt>
        <dd>{service.activeState} ({service.subState})</dd>
      </div>
      <div>
        <dt>{sinceLabel(service)}</dt>
        <dd>
          {#if service.since !== null}
            {formatDuration(serverNow - service.since)}
            <span class="sub">since <Time ts={service.since} /></span>
          {:else}
            Unknown
          {/if}
        </dd>
      </div>
      <div>
        <dt>Restarts</dt>
        <dd>{service.restarts}</dd>
      </div>
      <div>
        <dt>Memory</dt>
        <dd>{service.memory !== null ? formatBytes(service.memory) : 'Not reported'}</dd>
      </div>
      {#if service.loadState !== 'loaded'}
        <div>
          <dt>Unit file</dt>
          <dd>{service.loadState}</dd>
        </div>
      {/if}
      {#if service.description}
        <div class="wide">
          <dt>Description</dt>
          <dd>{service.description}</dd>
        </div>
      {/if}
    {/snippet}

    {#snippet children({ service, events })}
      <section class="history" aria-label="History">
        <div class="toolbar">
          <h2 class="heading">History</h2>
          <RangePicker
            selected={range.id}
            hrefFor={(id) => hrefWithQuery(['services', service.unit], { range: id })}
          />
        </div>
        <div class="charts">
          {#each chartGroups as group (group.id)}
            <ChartGroupPanel {group} {range} client={chartClient} {visibility} />
          {/each}
        </div>
      </section>

      <Section title="Events" count={events.length}>
        {#if events.length > 0}
          <CardList>
            {#each events as event (event.id)}
              <EventItem {event} {serverNow} showTarget={false} />
            {/each}
          </CardList>
          <a class="more" href={hrefWithQuery(['events'], { target: service.unit })}
            >All events for this service</a
          >
        {:else}
          <EmptyState title="No health changes recorded">
            Each change of this service's health is logged here.
          </EmptyState>
        {/if}
      </Section>
    {/snippet}
  </DetailState>
</div>

<style>
  .page {
    display: grid;
    gap: var(--space-6);
  }

  .wide {
    grid-column: 1 / -1;
  }

  .history {
    display: grid;
    gap: var(--space-3);
    min-width: 0;
  }

  .charts {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 28rem), 1fr));
    gap: var(--space-3);
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3) var(--space-5);
  }

  .heading {
    font-size: var(--text-md);
  }

  .more {
    align-self: flex-start;
    font-size: var(--text-sm);
  }
</style>
