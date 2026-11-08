<script lang="ts">
  import Time from '../components/Time.svelte';
  import { onMount, untrack } from 'svelte';
  import CardList from '../components/CardList.svelte';
  import DetailState from '../components/detail/DetailState.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import EventItem from '../components/events/EventItem.svelte';
  import Section from '../components/Section.svelte';
  import RunHistory from '../components/timers/RunHistory.svelte';
  import RunOutcome from '../components/timers/RunOutcome.svelte';
  import type { ApiClient } from '../lib/api/client';
  import { formatCountdown, formatDuration, formatRelativeTime } from '../lib/format';
  import { DETAIL_POLL_MS, PolledFeed } from '../lib/feed.svelte';
  import type { Visibility } from '../lib/poller';
  import { hrefWithQuery } from '../lib/router';
  import { isRunning, runDuration, runOutcome } from '../lib/timers';

  /** Runs to show; the backend's default, plenty to spot a pattern. */
  const RUN_LIMIT = 50;

  // App re-creates the page when `unit` changes, so it is read once.
  let {
    unit,
    client,
    visibility,
    serverNow,
  }: {
    unit: string;
    client: Pick<ApiClient, 'timer'>;
    visibility?: Visibility;
    serverNow: number;
  } = $props();

  const feed = new PolledFeed(
    (signal) =>
      client.timer(
        untrack(() => unit),
        { limit: RUN_LIMIT },
        { signal },
      ),
    { intervalMs: DETAIL_POLL_MS, visibility: untrack(() => visibility) },
  );

  onMount(() => {
    feed.start();
    return () => feed.stop();
  });
</script>

<div class="page">
  <DetailState
    data={feed.data}
    error={feed.error}
    noun="timer"
    kind="Timer"
    runs
    name={unit}
    onretry={() => feed.refresh()}
    summary={({ timer }) => ({
      kind: 'Timer',
      name: timer.name,
      unit: timer.unit,
      health: timer.health,
      message: timer.message,
    })}
  >
    {#snippet facts({ timer })}
      <div>
        <dt>Last run</dt>
        <dd>
          {#if timer.lastRun}
            <Time
              ts={timer.lastRun.startedAt}
              text={formatRelativeTime(timer.lastRun.startedAt, serverNow)}
            />
            <span class="sub"><RunOutcome run={timer.lastRun} /></span>
          {:else}
            Timer has never run
          {/if}
        </dd>
      </div>
      {#if timer.lastRun}
        <div>
          <dt>{runOutcome(timer.lastRun) === 'running' ? 'Running for' : 'Took'}</dt>
          <dd>{formatDuration(runDuration(timer.lastRun, serverNow))}</dd>
        </div>
      {/if}
      <div>
        <dt>Next run</dt>
        <dd>
          {#if timer.nextRun !== null}
            <Time ts={timer.nextRun} text={formatCountdown(timer.nextRun, serverNow)} />
            <span class="sub"><Time ts={timer.nextRun} /></span>
          {:else}
            Not scheduled
          {/if}
        </dd>
      </div>
      <div>
        <dt>Timer</dt>
        <dd>{timer.active ? 'Active' : 'Inactive'}{isRunning(timer) ? ', running now' : ''}</dd>
      </div>
      <div>
        <dt>Runs service</dt>
        <dd><code>{timer.service}</code></dd>
      </div>
    {/snippet}

    {#snippet children({ timer, runs, events })}
      <Section title="Run history" count={runs.length}>
        {#if runs.length > 0}
          <RunHistory {runs} {serverNow} />
        {:else}
          <EmptyState title="No runs recorded yet">
            {timer.lastRun === null
              ? 'Timer has never run.'
              : 'Runs appear here once Gargoyle has observed one.'}
          </EmptyState>
        {/if}
      </Section>

      <Section title="Events" count={events.length}>
        {#if events.length > 0}
          <CardList>
            {#each events as event (event.id)}
              <EventItem {event} {serverNow} showTarget={false} />
            {/each}
          </CardList>
          <a class="more" href={hrefWithQuery(['events'], { target: timer.unit })}
            >All events for this timer</a
          >
        {:else}
          <EmptyState title="No health changes recorded">
            Each change of this timer's health is logged here.
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

  .more {
    align-self: flex-start;
    font-size: var(--text-sm);
  }
</style>
