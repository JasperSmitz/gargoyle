<script lang="ts" generics="T">
  import type { Snippet } from 'svelte';
  import { ApiRequestError } from '../../lib/api/client';
  import { describeFeedError } from '../../lib/overview-feed.svelte';
  import { href } from '../../lib/router';
  import EmptyState from '../EmptyState.svelte';

  // Loading, not-found and error states for a detail page's polled data. Once data has arrived it
  // stays on screen, with a note if a later refresh fails.
  let {
    data,
    error,
    noun,
    name,
    onretry,
    children,
  }: {
    data: T | null;
    error: unknown;
    /** What the page shows, e.g. "timer". */
    noun: string;
    /** The requested unit, for the not-found message. */
    name: string;
    onretry: () => void;
    children: Snippet<[T]>;
  } = $props();

  const notFound = $derived(error instanceof ApiRequestError && error.status === 404);
  const failure = $derived(error === null ? null : describeFeedError(error));
</script>

{#if notFound}
  <EmptyState title="No {noun} called {name}">
    Only units configured in <code>gargoyle.toml</code> are watched.
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
  <p class="loading" role="status">Loading {noun}…</p>
{/if}

<style>
  .note {
    font-size: var(--text-sm);
    color: var(--health-warning-fg);
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
