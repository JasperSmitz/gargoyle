<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import AppHeader from './components/AppHeader.svelte';
  import EmptyState from './components/EmptyState.svelte';
  import OverviewSkeleton from './components/OverviewSkeleton.svelte';
  import StaleBanner from './components/StaleBanner.svelte';
  import { api, type ApiClient } from './lib/api/client';
  import type { MetricsQuery } from './lib/api/types';
  import { averagesByKey } from './lib/charts/data';
  import { buildQuery } from './lib/charts/groups';
  import { parseRange, rangeById } from './lib/charts/range';
  import { PolledFeed } from './lib/feed.svelte';
  import { OverviewFeed, describeFeedError } from './lib/overview-feed.svelte';
  import type { Visibility } from './lib/poller';
  import { hashQuery, href, parseHash } from './lib/router';
  import OverviewPage from './pages/OverviewPage.svelte';
  import SystemPage from './pages/SystemPage.svelte';

  // Props exist for tests; the app itself mounts App with none.
  let {
    client = api,
    visibility,
  }: {
    client?: Pick<ApiClient, 'overview' | 'metrics' | 'metricsQuery'>;
    visibility?: Visibility;
  } = $props();

  // The header needs overview data on every page, so the feed lives here rather than in a page.
  // Test-only props are read once on purpose; they never change after mount.
  const feed = new OverviewFeed((signal) => client.overview({ signal }), {
    visibility: untrack(() => visibility),
  });

  // Overview sparklines: the last hour of the carded metrics, one request, refreshed every 60s.
  const SPARKLINE_KEYS = ['cpu.usage', 'load.1', 'mem.used', 'disk.used:*'];
  const SPARKLINE_POINTS = 60;
  const sparklines = new PolledFeed<MetricsQuery>(
    (signal) =>
      client.metricsQuery(
        buildQuery(SPARKLINE_KEYS, rangeById('1h'), Date.now() / 1000, SPARKLINE_POINTS),
        { signal },
      ),
    { visibility: untrack(() => visibility) },
  );
  const history = $derived(sparklines.data ? averagesByKey(sparklines.data) : null);

  let hash = $state(location.hash);
  const route = $derived(parseHash(hash));
  const range = $derived(parseRange(hashQuery(hash).get('range')));
  let now = $state(Date.now() / 1000);

  // Keyed on the name only: a new route object for the same page must not restart the feed.
  const routeName = $derived(route.name);
  $effect(() => {
    if (routeName !== 'overview') return;
    sparklines.start();
    return () => sparklines.stop();
  });

  onMount(() => {
    feed.start();
    // Relative times ("updated 5s ago") tick between polls.
    const tick = setInterval(() => (now = Date.now() / 1000), 1000);
    return () => {
      feed.stop();
      clearInterval(tick);
    };
  });

  const failure = $derived(feed.error === null ? null : describeFeedError(feed.error));
</script>

<svelte:window onhashchange={() => (hash = location.hash)} />

<AppHeader
  host={feed.data?.host ?? null}
  health={feed.data?.health ?? null}
  receivedAt={feed.receivedAt}
  {now}
  stale={failure !== null}
  route={route.name}
/>

<main class="main">
  {#if failure}
    <StaleBanner
      title={failure.title}
      detail={failure.detail}
      receivedAt={feed.receivedAt}
      {now}
      onretry={() => feed.refresh()}
    />
  {/if}

  {#if route.name === 'overview'}
    {#if feed.data}
      <OverviewPage overview={feed.data} serverNow={now + feed.clockOffset} {history} />
    {:else if !failure}
      <OverviewSkeleton />
    {/if}
  {:else if route.name === 'system'}
    <SystemPage {range} {client} {visibility} />
  {:else}
    <EmptyState title="Page not found">
      Nothing lives at <code>{route.path}</code>. <a href={href()}>Back to the overview</a>
    </EmptyState>
  {/if}
</main>

<style>
  .main {
    display: grid;
    gap: var(--space-5);
    max-width: var(--content-max-width);
    margin: 0 auto;
    padding: var(--space-5) var(--space-4) var(--space-7);
  }
</style>
