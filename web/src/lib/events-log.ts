// Paging through `/events` (newest first) with the `until` cursor.
//
// Contract (coordinator decision for M3.5): `until` is exclusive, a page always holds whole
// seconds (so it may be longer or shorter than `limit`), and an empty page marks the end of the
// log. The next cursor is therefore the oldest timestamp seen. Pages are still merged by event
// id, since a poll of the newest page overlaps what is already loaded.

import type { HealthEvent, UnixSeconds } from './api/types';

export const EVENTS_PAGE_SIZE = 50;

export interface EventsLog {
  /** Newest first, unique by id. */
  events: HealthEvent[];
  /** `until` for the next older page; null once an empty page showed nothing older exists. */
  cursor: UnixSeconds | null;
}

export const EMPTY_LOG: EventsLog = { events: [], cursor: null };

/** Newest first; events in the same second by descending id, matching insertion order. */
function byRecency(a: HealthEvent, b: HealthEvent): number {
  return b.ts - a.ts || b.id - a.id;
}

function union(existing: readonly HealthEvent[], page: readonly HealthEvent[]): HealthEvent[] {
  const seen = new Set(existing.map((e) => e.id));
  return [...existing, ...page.filter((e) => !seen.has(e.id))].sort(byRecency);
}

/** Adds an older page. Only an empty page ends the log. */
export function appendOlder(log: EventsLog, page: readonly HealthEvent[]): EventsLog {
  if (page.length === 0) return { events: log.events, cursor: null };
  const events = union(log.events, page);
  return { events, cursor: events.at(-1)?.ts ?? null };
}

/**
 * Merges the newest page (the first load, then each poll) without touching the cursor. If a
 * page shares nothing with what is loaded, there may be unseen events between the two, so the
 * log restarts from that page rather than show a hole.
 */
export function mergeNewest(log: EventsLog, page: readonly HealthEvent[]): EventsLog {
  if (log.events.length === 0) return appendOlder(EMPTY_LOG, page);
  if (page.length === 0) return log;
  const known = new Set(log.events.map((e) => e.id));
  if (!page.some((e) => known.has(e.id))) return appendOlder(EMPTY_LOG, page);
  return { events: union(log.events, page), cursor: log.cursor };
}
