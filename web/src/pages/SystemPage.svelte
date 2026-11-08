<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import ChartGroupPanel from '../components/charts/ChartGroupPanel.svelte';
  import RangePicker from '../components/charts/RangePicker.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import type { ApiClient } from '../lib/api/client';
  import { buildChartGroups } from '../lib/charts/groups';
  import type { Range } from '../lib/charts/range';
  import { PolledFeed } from '../lib/feed.svelte';
  import { describeFeedError } from '../lib/overview-feed.svelte';
  import type { Visibility } from '../lib/poller';
  import { hrefWithQuery } from '../lib/router';

  let {
    range,
    client,
    visibility,
  }: {
    range: Range;
    client: Pick<ApiClient, 'metrics' | 'metricsQuery'>;
    visibility?: Visibility;
  } = $props();

  // The series list decides which charts exist; refreshed with the charts in case sensors or
  // interfaces appear later.
  const catalog = new PolledFeed((signal) => client.metrics({ signal }), {
    visibility: untrack(() => visibility),
  });

  onMount(() => {
    catalog.start();
    return () => catalog.stop();
  });

  const groups = $derived(catalog.data ? buildChartGroups(catalog.data.series) : []);
  const failure = $derived(catalog.error === null ? null : describeFeedError(catalog.error));
</script>

<div class="page">
  <div class="toolbar">
    <h1 class="heading">System</h1>
    <RangePicker selected={range.id} hrefFor={(id) => hrefWithQuery(['system'], { range: id })} />
  </div>

  {#if catalog.data}
    {#if groups.length > 0}
      <div class="charts">
        {#each groups as group (group.id + group.queryKeys.join())}
          <ChartGroupPanel {group} {range} {client} {visibility} />
        {/each}
      </div>
    {:else}
      <EmptyState title="No metrics recorded yet">
        Charts appear once the server has stored its first samples.
      </EmptyState>
    {/if}
  {:else if failure}
    <EmptyState title="Could not load the list of metrics">
      {failure.detail}.
      <button type="button" class="retry" onclick={() => catalog.refresh()}>Try again</button>
    </EmptyState>
  {:else}
    <p class="loading" role="status">Loading metrics…</p>
  {/if}
</div>

<style>
  .page {
    display: grid;
    gap: var(--space-4);
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3) var(--space-5);
  }

  .heading {
    font-size: var(--text-xl);
  }

  .charts {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 28rem), 1fr));
    gap: var(--space-3);
  }

  .loading {
    color: var(--color-text-faint);
  }

  .retry {
    padding: 0;
    color: var(--color-accent);
    background: none;
    border: 0;
    text-decoration: underline;
    cursor: pointer;
  }
</style>
