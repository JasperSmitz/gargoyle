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
  import type { Range } from '../lib/charts/range';
  import { CHECK_KIND_LABEL, checkChartGroups, formatLatency } from '../lib/checks';
  import { DETAIL_POLL_MS, PolledFeed } from '../lib/feed.svelte';
  import { formatRelativeTime } from '../lib/format';
  import type { Visibility } from '../lib/poller';
  import { hrefWithQuery } from '../lib/router';
  import { treatUnknownKeyAsEmpty } from '../lib/service-charts';

  // App re-creates the page when `name` changes, so it is read once. There is no latency
  // threshold line: the contract does not expose `latency_warning`, only the resulting health.
  let {
    name,
    range,
    client,
    visibility,
    serverNow,
  }: {
    name: string;
    range: Range;
    client: Pick<ApiClient, 'check' | 'metricsQuery' | 'metrics'>;
    visibility?: Visibility;
    serverNow: number;
  } = $props();

  const feed = new PolledFeed(
    (signal) =>
      client.check(
        untrack(() => name),
        { signal },
      ),
    { intervalMs: DETAIL_POLL_MS, visibility: untrack(() => visibility) },
  );
  const chartGroups = checkChartGroups(untrack(() => name));
  // A check that has never answered has no latency series yet: "no data", not an error.
  const chartClient = treatUnknownKeyAsEmpty(untrack(() => client));

  onMount(() => {
    feed.start();
    return () => feed.stop();
  });
</script>

<div class="page">
  <DetailState
    data={feed.data}
    error={feed.error}
    noun="check"
    kind="Check"
    charts={chartGroups.length}
    {name}
    onretry={() => feed.refresh()}
    summary={({ check }) => ({
      kind: `${CHECK_KIND_LABEL[check.kind]} check`,
      name: check.name,
      unit: check.target,
      health: check.health,
      message: check.message,
    })}
  >
    {#snippet facts({ check })}
      {@const latency = formatLatency(check)}
      <div>
        <dt>Last check</dt>
        <dd>
          {#if check.lastCheck !== null}
            <Time ts={check.lastCheck} text={formatRelativeTime(check.lastCheck, serverNow)} />
            <span class="sub"><Time ts={check.lastCheck} /></span>
          {:else}
            Not run yet
          {/if}
        </dd>
      </div>
      <div>
        <dt>Latency</dt>
        <dd>
          {#if latency !== null}
            {latency}
          {:else}
            {check.lastCheck === null ? 'Not run yet' : 'Failed'}
          {/if}
        </dd>
      </div>
      <div>
        <dt>Consecutive failures</dt>
        <dd>
          {check.consecutiveFailures}
          {#if check.consecutiveFailures > 0}<span class="sub">in a row</span>{/if}
        </dd>
      </div>
    {/snippet}

    {#snippet children({ check, events })}
      <section class="history" aria-label="History">
        <div class="toolbar">
          <h2 class="heading">History</h2>
          <RangePicker
            selected={range.id}
            hrefFor={(id) => hrefWithQuery(['checks', check.name], { range: id })}
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
          <a class="more" href={hrefWithQuery(['events'], { kind: 'check', target: check.name })}
            >All events for this check</a
          >
        {:else}
          <EmptyState title="No health changes recorded">
            Each change of this check's health is logged here.
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

  .history {
    display: grid;
    gap: var(--space-3);
    min-width: 0;
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
