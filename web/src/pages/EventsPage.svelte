<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import CardList from '../components/CardList.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import EventItem from '../components/events/EventItem.svelte';
  import ListSkeleton from '../components/skeleton/ListSkeleton.svelte';
  import type { ApiClient } from '../lib/api/client';
  import { TARGET_KINDS, type Overview, type TargetKind } from '../lib/api/types';
  import { targetOptions, type EventFilters } from '../lib/event-filters';
  import { EventsFeed } from '../lib/events-feed.svelte';
  import { describeFeedError } from '../lib/overview-feed.svelte';
  import type { Visibility } from '../lib/poller';
  import { hrefWithQuery } from '../lib/router';

  // Both filters go to the API; App re-creates the page when either changes.
  let {
    filters,
    client,
    overview,
    visibility,
    serverNow,
    pageSize,
  }: {
    filters: EventFilters;
    client: Pick<ApiClient, 'events'>;
    /** For the target list; null before the first overview. */
    overview: Overview | null;
    visibility?: Visibility;
    serverNow: number;
    /** Test-only override of the page size. */
    pageSize?: number;
  } = $props();

  const feed = new EventsFeed(
    async (query, signal) => {
      const { target, kind } = untrack(() => filters);
      const response = await client.events(
        { ...query, target: target ?? undefined, kind: kind ?? undefined },
        { signal },
      );
      return response.events;
    },
    { visibility: untrack(() => visibility), pageSize: untrack(() => pageSize) },
  );

  onMount(() => {
    feed.start();
    return () => feed.stop();
  });

  const KIND_LABELS: Record<TargetKind, string> = {
    system: 'System',
    service: 'Services',
    timer: 'Timers',
    check: 'Checks',
    library: 'Library',
  };

  const allOptions = $derived(targetOptions(overview, feed.log.events));
  const options = $derived(
    allOptions.filter((o) => filters.kind === null || o.kind === filters.kind),
  );
  const failure = $derived(feed.error === null ? null : describeFeedError(feed.error));
  const olderFailure = $derived(
    feed.olderError === null ? null : describeFeedError(feed.olderError),
  );

  function navigate(next: EventFilters) {
    location.hash = hrefWithQuery(['events'], {
      kind: next.kind ?? undefined,
      target: next.target ?? undefined,
    });
  }

  function onKind(value: string) {
    const kind = TARGET_KINDS.find((k) => k === value) ?? null;
    // A target of another kind would make the list empty by construction; drop it.
    const targetKind = allOptions.find((o) => o.target === filters.target)?.kind;
    const keep = kind === null || targetKind === undefined || targetKind === kind;
    navigate({ kind, target: keep ? filters.target : null });
  }

  const filtered = $derived(filters.kind !== null || filters.target !== null);
  const emptyTitle = $derived(
    filters.target !== null
      ? `No events for ${filters.target}`
      : filters.kind !== null
        ? `No ${filters.kind} events yet`
        : 'No events yet',
  );
</script>

<div class="page">
  <div class="toolbar">
    <h1 class="heading">Events</h1>
    <form class="filters" aria-label="Filter events" onsubmit={(e) => e.preventDefault()}>
      <label>
        <span>Kind</span>
        <select
          name="kind"
          value={filters.kind ?? ''}
          onchange={(e) => onKind(e.currentTarget.value)}
        >
          <option value="">All kinds</option>
          {#each TARGET_KINDS as kind (kind)}
            <option value={kind}>{KIND_LABELS[kind]}</option>
          {/each}
        </select>
      </label>
      <label>
        <span>Target</span>
        <select
          name="target"
          value={filters.target ?? ''}
          onchange={(e) => navigate({ kind: filters.kind, target: e.currentTarget.value || null })}
        >
          <option value="">All targets</option>
          {#if filters.target !== null && !options.some((o) => o.target === filters.target)}
            <option value={filters.target}>{filters.target}</option>
          {/if}
          {#each options as option (option.target)}
            <option value={option.target}>{option.target}</option>
          {/each}
        </select>
      </label>
      {#if filtered}
        <a class="clear" href={hrefWithQuery(['events'], {})}>Clear filters</a>
      {/if}
    </form>
  </div>

  {#if feed.loaded}
    {#if failure}
      <p class="note" role="alert">Refresh failed: {failure.detail}.</p>
    {/if}
    {#if feed.log.events.length > 0}
      <CardList label="Events">
        {#each feed.log.events as event (event.id)}
          <EventItem {event} {serverNow} showTarget={filters.target === null} />
        {/each}
      </CardList>
    {:else}
      <EmptyState title={emptyTitle}>
        Every health change of a watched target is recorded here.
      </EmptyState>
    {/if}

    <div class="paging">
      {#if feed.log.cursor !== null}
        <button
          type="button"
          class="older"
          disabled={feed.loadingOlder}
          onclick={() => feed.loadOlder()}
        >
          {feed.loadingOlder ? 'Loading…' : 'Load older events'}
        </button>
      {:else if feed.log.events.length > 0}
        <p class="end">Start of the log</p>
      {/if}
      {#if olderFailure}
        <p class="note" role="alert">Could not load older events: {olderFailure.detail}.</p>
      {/if}
    </div>
  {:else if failure}
    <EmptyState title="Could not load events">
      {failure.detail}.
      <button type="button" class="retry" onclick={() => feed.refresh()}>Try again</button>
    </EmptyState>
  {:else}
    <ListSkeleton rows={8} label="Loading events" />
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
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--space-3) var(--space-5);
  }

  .heading {
    font-size: var(--text-xl);
  }

  .filters {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--space-2) var(--space-3);
  }

  label {
    display: grid;
    gap: 2px;
    font-size: var(--text-xs);
    color: var(--color-text-faint);
  }

  select {
    min-width: 9rem;
    max-width: 100%;
    padding: var(--space-1) var(--space-2);
    font: inherit;
    font-size: var(--text-sm);
    color: var(--color-text);
    background: var(--color-surface-raised);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-sm);
  }

  .clear {
    padding-block: var(--space-1);
    font-size: var(--text-sm);
  }

  .paging {
    display: grid;
    justify-items: center;
    gap: var(--space-2);
  }

  .older {
    padding: var(--space-2) var(--space-4);
    font-size: var(--text-sm);
    color: var(--color-text);
    background: var(--color-surface-raised);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    cursor: pointer;
  }

  .older:hover:not(:disabled) {
    background: var(--color-surface-sunken);
  }

  .older:disabled {
    color: var(--color-text-muted);
    cursor: progress;
  }

  .end {
    font-size: var(--text-sm);
    color: var(--color-text-faint);
  }

  .note {
    font-size: var(--text-sm);
    color: var(--health-warning-fg);
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
