<script lang="ts" module>
  import type { Health } from '../../lib/api/types';

  /** What DetailHeader shows once the detail has loaded. */
  export interface DetailSummary {
    kind: string;
    name: string;
    unit: string;
    health: Health;
    message: string | null;
  }
</script>

<script lang="ts" generics="T">
  import type { Snippet } from 'svelte';
  import { ApiRequestError } from '../../lib/api/client';
  import { describeFeedError } from '../../lib/overview-feed.svelte';
  import { href } from '../../lib/router';
  import EmptyState from '../EmptyState.svelte';
  import DetailSkeleton from '../skeleton/DetailSkeleton.svelte';
  import DetailHeader from './DetailHeader.svelte';

  // Header plus loading, not-found and error states for a detail page's polled data. The header
  // is rendered once, outside the state switch, so its h1 (the route focus target) survives the
  // first response. Once data has arrived it stays on screen, with a note if a refresh fails.
  let {
    data,
    error,
    noun,
    kind,
    name,
    charts = 0,
    runs = false,
    onretry,
    summary,
    facts: factsFor,
    children,
  }: {
    data: T | null;
    error: unknown;
    /** What the page shows, e.g. "timer". */
    noun: string;
    /** Header label until the detail has loaded, e.g. "Timer". */
    kind: string;
    /** The requested unit: the heading until the detail names it, and the not-found message. */
    name: string;
    /** Chart panels the loaded page shows, so the skeleton reserves their space. */
    charts?: number;
    /** The loaded page shows a run history list. */
    runs?: boolean;
    onretry: () => void;
    summary: (data: T) => DetailSummary;
    /** The header's facts (`<div><dt/><dd/></div>` groups). */
    facts: Snippet<[T]>;
    /** Everything below the header. */
    children: Snippet<[T]>;
  } = $props();

  const notFound = $derived(error instanceof ApiRequestError && error.status === 404);
  const failure = $derived(error === null ? null : describeFeedError(error));
  const shown = $derived(data === null ? null : summary(data));
</script>

<DetailHeader
  kind={shown?.kind ?? kind}
  name={shown?.name ?? name}
  unit={shown?.unit ?? name}
  health={shown?.health ?? null}
  message={shown?.message ?? null}
  loading={data === null && error === null ? `Loading ${noun}` : null}
>
  {#snippet facts()}{#if data !== null}{@render factsFor(data)}{/if}{/snippet}
</DetailHeader>

{#if notFound}
  <EmptyState title="No {noun} called {name}">
    Only targets configured in <code>gargoyle.toml</code> are watched.
    <a href={href()}>Back to the overview</a>
  </EmptyState>
{:else if data !== null}
  {#if failure}
    <p class="note" role="alert">
      Refresh failed: {failure.detail}. Showing the last data received.
    </p>
  {/if}
  {@render children(data)}
{:else if failure}
  <EmptyState title="Could not load this {noun}">
    {failure.detail}.
    <button type="button" class="retry" onclick={onretry}>Try again</button>
  </EmptyState>
{:else}
  <DetailSkeleton {noun} {charts} {runs} />
{/if}

<style>
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
