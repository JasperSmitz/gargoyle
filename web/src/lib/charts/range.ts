// Time ranges for the range picker. The choice lives in the hash query (`#/system?range=7d`) so
// a link or reload keeps it.

import type { UnixSeconds } from '../api/types';

const HOUR = 3600;
const DAY = 24 * HOUR;

export const RANGES = [
  { id: '1h', seconds: HOUR, description: 'last hour' },
  { id: '6h', seconds: 6 * HOUR, description: 'last 6 hours' },
  { id: '24h', seconds: DAY, description: 'last 24 hours' },
  { id: '7d', seconds: 7 * DAY, description: 'last 7 days' },
  { id: '30d', seconds: 30 * DAY, description: 'last 30 days' },
  { id: '1y', seconds: 365 * DAY, description: 'last year' },
] as const;

export type Range = (typeof RANGES)[number];
export type RangeId = Range['id'];

export const DEFAULT_RANGE: RangeId = '24h';

/** The range named in the URL, or the default for a missing or unknown value. */
export function parseRange(value: string | null | undefined): Range {
  return RANGES.find((r) => r.id === value) ?? rangeById(DEFAULT_RANGE);
}

export function rangeById(id: RangeId): Range {
  // RANGES is a closed literal list and `id` is one of its ids, so the lookup cannot miss.
  return RANGES.find((r) => r.id === id) as Range;
}

/** The query window ending now: whole seconds, `to` inclusive of the latest sample. */
export function rangeWindow(range: Range, now: number): { from: UnixSeconds; to: UnixSeconds } {
  const to = Math.floor(now);
  return { from: to - range.seconds, to };
}
