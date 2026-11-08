import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { HealthEvent } from './api/types';
import { EventsFeed, type EventsPageLoader } from './events-feed.svelte';
import type { Visibility } from './poller';

const visible: Visibility = { isHidden: () => false, subscribe: () => () => {} };

const ev = (id: number, ts: number): HealthEvent => ({
  id,
  ts,
  targetKind: 'check',
  target: 'Music HTTP',
  from: 'healthy',
  to: 'warning',
  message: null,
});

/** A loader whose responses the test hands out one by one, in any order. */
function deferredLoader() {
  const calls: { query: Parameters<EventsPageLoader>[0]; resolve: (p: HealthEvent[]) => void }[] =
    [];
  const load = vi.fn<EventsPageLoader>(
    (query) => new Promise((resolve) => calls.push({ query, resolve })),
  );
  return { load, calls };
}

const ids = (feed: EventsFeed) => feed.log.events.map((e) => e.id);

describe('EventsFeed', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  async function started() {
    const { load, calls } = deferredLoader();
    const feed = new EventsFeed(load, { visibility: visible, intervalMs: 30_000 });
    feed.start();
    calls[0]!.resolve([ev(10, 100), ev(9, 90)]);
    await vi.advanceTimersByTimeAsync(0);
    return { feed, calls };
  }

  it('appends an older page to the log it continues', async () => {
    const { feed, calls } = await started();
    const older = feed.loadOlder();
    expect(calls[1]?.query).toEqual({ until: 90, limit: 50 });
    calls[1]!.resolve([ev(8, 80)]);
    await older;
    expect(ids(feed)).toEqual([10, 9, 8]);
    expect(feed.log.cursor).toBe(80);
  });

  it('drops an older page that arrives after a poll restarted the log', async () => {
    const { feed, calls } = await started();
    const older = feed.loadOlder();
    expect(feed.loadingOlder).toBe(true);

    // More than a page of new events: the poll shares no id with the log, so it restarts.
    await vi.advanceTimersByTimeAsync(30_000);
    calls[2]!.resolve([ev(20, 200), ev(19, 190)]);
    await vi.advanceTimersByTimeAsync(0);
    expect(ids(feed)).toEqual([20, 19]);
    expect(feed.loadingOlder).toBe(false);

    // The stale response (ts < 90) would leave everything between 90 and 190 out.
    calls[1]!.resolve([ev(8, 80)]);
    await older;
    expect(ids(feed)).toEqual([20, 19]);
    expect(feed.log.cursor).toBe(190);
    expect(feed.olderError).toBeNull();

    // Paging continues from the new log.
    void feed.loadOlder();
    expect(calls[3]?.query).toEqual({ until: 190, limit: 50 });
  });

  it('keeps an older page in flight across a poll that only adds new events', async () => {
    const { feed, calls } = await started();
    const older = feed.loadOlder();
    await vi.advanceTimersByTimeAsync(30_000);
    calls[2]!.resolve([ev(11, 110), ev(10, 100)]);
    await vi.advanceTimersByTimeAsync(0);
    calls[1]!.resolve([ev(8, 80)]);
    await older;
    expect(ids(feed)).toEqual([11, 10, 9, 8]);
  });

  it('ends the log on an empty older page', async () => {
    const { feed, calls } = await started();
    const older = feed.loadOlder();
    calls[1]!.resolve([]);
    await older;
    expect(feed.log.cursor).toBeNull();
    await feed.loadOlder();
    expect(calls).toHaveLength(2);
  });
});
