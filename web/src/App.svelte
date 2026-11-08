<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import AppHeader from './components/AppHeader.svelte';
  import EmptyState from './components/EmptyState.svelte';
  import OverviewSkeleton from './components/OverviewSkeleton.svelte';
  import StaleBanner from './components/StaleBanner.svelte';
  import { api, type ApiClient } from './lib/api/client';
  import { OverviewFeed, describeFeedError } from './lib/overview-feed.svelte';
  import type { Visibility } from './lib/poller';
  import { href, parseHash } from './lib/router';
  import OverviewPage from './pages/OverviewPage.svelte';

  // Props exist for tests; the app itself mounts App with none.
  let {
    client = api,
    visibility,
  }: { client?: Pick<ApiClient, 'overview'>; visibility?: Visibility } = $props();

  // The header needs overview data on every page, so the feed lives here rather than in a page.
  // Test-only props are read once on purpose; they never change after mount.
  const feed = new OverviewFeed((signal) => client.overview({ signal }), {
    visibility: untrack(() => visibility),
  });

  let route = $state(parseHash(location.hash));
  let now = $state(Date.now() / 1000);

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

<svelte:window onhashchange={() => (route = parseHash(location.hash))} />

<AppHeader
  host={feed.data?.host ?? null}
  health={feed.data?.health ?? null}
  receivedAt={feed.receivedAt}
  {now}
  stale={failure !== null}
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
      <OverviewPage overview={feed.data} serverNow={now + feed.clockOffset} />
    {:else if !failure}
      <OverviewSkeleton />
    {/if}
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
