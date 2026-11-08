import { describe, expect, it } from 'vitest';
import type { HealthEvent } from './api/types';
import { EMPTY_LOG, appendOlder, mergeNewest } from './events-log';

/** Events with ids 1..n, newest first; `perSecond` events share each timestamp. */
function history(n: number, perSecond = 1): HealthEvent[] {
  return Array.from({ length: n }, (_, i) => {
    const id = n - i;
    return {
      id,
      ts: Math.ceil(id / perSecond) * 10,
      targetKind: 'timer',
      target: 'a.timer',
      from: 'healthy',
      to: 'critical',
      message: null,
    } satisfies HealthEvent;
  });
}

/** The backend: exclusive `until`, whole seconds per page (may exceed `limit`). */
function backend(all: HealthEvent[]) {
  return (until: number | null, limit: number) => {
    const matching = all.filter((e) => until === null || e.ts < until);
    const last = matching[limit - 1];
    return last === undefined ? matching : matching.filter((e, i) => i < limit || e.ts === last.ts);
  };
}

function walk(all: HealthEvent[], limit: number) {
  const fetchPage = backend(all);
  const requests: (number | null)[] = [null];
  let log = mergeNewest(EMPTY_LOG, fetchPage(null, limit));
  while (log.cursor !== null && requests.length < 50) {
    requests.push(log.cursor);
    log = appendOlder(log, fetchPage(log.cursor, limit));
  }
  return { log, requests };
}

describe('events paging', () => {
  it.each([1, 3, 7])('walks the whole log exactly once (%d events per second)', (perSecond) => {
    const all = history(23, perSecond);
    const { log } = walk(all, 5);
    expect(log.events.map((e) => e.id)).toEqual(all.map((e) => e.id));
    expect(log.cursor).toBeNull();
  });

  it('keeps going after a short page and stops only at an empty one', () => {
    const all = history(12);
    const { requests, log } = walk(all, 5);
    // 5 + 5 + 2 (short, but not the end) + empty.
    expect(requests).toEqual([null, all[4]!.ts, all[9]!.ts, all[11]!.ts]);
    expect(log.events).toHaveLength(12);
  });

  it('puts the cursor at the oldest event seen', () => {
    expect(mergeNewest(EMPTY_LOG, history(3)).cursor).toBe(10);
  });

  it('ends at once for an empty log', () => {
    expect(mergeNewest(EMPTY_LOG, [])).toEqual({ events: [], cursor: null });
  });

  it('orders same-second events by id and drops duplicates', () => {
    const [a, b] = history(2).map((e) => ({ ...e, ts: 50 }));
    const log = appendOlder({ events: [a!], cursor: 50 }, [b!, a!]);
    expect(log.events.map((e) => e.id)).toEqual([2, 1]);
  });
});

describe('mergeNewest (polling)', () => {
  it('adds new events on top and keeps the cursor', () => {
    const all = history(30);
    const loaded = { events: all.slice(5, 15), cursor: 100 };
    const log = mergeNewest(loaded, all.slice(0, 10));
    expect(log.events.map((e) => e.id)).toEqual(all.slice(0, 15).map((e) => e.id));
    expect(log.cursor).toBe(100);
  });

  it('keeps what is loaded when a poll returns nothing', () => {
    const loaded = { events: history(3), cursor: null };
    expect(mergeNewest(loaded, [])).toBe(loaded);
  });

  it('restarts when the new page does not touch what is loaded, rather than leave a hole', () => {
    const all = history(40);
    const loaded = { events: all.slice(25), cursor: null };
    const log = mergeNewest(loaded, all.slice(0, 10));
    expect(log.events.map((e) => e.id)).toEqual(all.slice(0, 10).map((e) => e.id));
    expect(log.cursor).toBe(all[9]?.ts);
  });
});
