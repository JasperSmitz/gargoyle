<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import type { ApiClient } from '../../lib/api/client';
  import type { MetricsQuery } from '../../lib/api/types';
  import { alignSeries } from '../../lib/charts/data';
  import { buildQuery, type ChartGroup } from '../../lib/charts/groups';
  import type { Range } from '../../lib/charts/range';
  import { PolledFeed } from '../../lib/feed.svelte';
  import { describeFeedError } from '../../lib/overview-feed.svelte';
  import type { Visibility } from '../../lib/poller';
  import TimeSeriesChart from './TimeSeriesChart.svelte';

  // One query per group, polled every 60s, so each group loads and fails on its own.
  let {
    group,
    range,
    client,
    visibility,
  }: {
    group: ChartGroup;
    range: Range;
    client: Pick<ApiClient, 'metricsQuery'>;
    visibility?: Visibility;
  } = $props();

  // Each poll reads the current range and keys, so a refresh is all a change needs.
  const feed = new PolledFeed<{ range: Range; response: MetricsQuery }>(
    async (signal) => {
      const asked = range;
      const query = buildQuery(group.queryKeys, asked, Date.now() / 1000);
      return { range: asked, response: await client.metricsQuery(query, { signal }) };
    },
    { visibility: untrack(() => visibility) },
  );

  onMount(() => {
    feed.start();
    return () => feed.stop();
  });

  let shownRange = untrack(() => range.id);
  $effect(() => {
    if (range.id === shownRange) return;
    shownRange = range.id;
    untrack(() => feed.refresh());
  });

  const error = $derived(feed.error === null ? null : describeFeedError(feed.error).detail);
</script>

{#each group.charts as chart (chart.id)}
  <TimeSeriesChart
    title={chart.title}
    unit={chart.unit}
    series={chart.series}
    data={feed.data
      ? alignSeries(
          feed.data.response,
          chart.series.map((s) => s.key),
        )
      : null}
    loading={feed.loading}
    {error}
    rangeDescription={(feed.data?.range ?? range).description}
  />
{/each}
