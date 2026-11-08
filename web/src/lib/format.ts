// Display formatting for API values. Inputs follow the contract: bytes, percent (0–100),
// seconds and unix seconds. Every formatter returns a placeholder instead of `NaN`/`Infinity`.

export const PLACEHOLDER = '—';

const BYTE_UNITS = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'] as const;

/** IEC (1024-based) sizes, matching what `free -h` and `df -h` print: `15.5 GiB`, `233 GiB`. */
export function formatBytes(bytes: number | null): string {
  if (bytes === null || !Number.isFinite(bytes) || bytes < 0) return PLACEHOLDER;
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < BYTE_UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  if (unit === 0) return `${Math.round(value)} B`;
  // Three significant figures at most; rounding up to 1024 would read oddly, so step a unit up.
  if (value >= 1023.5 && unit < BYTE_UNITS.length - 1) return `1.0 ${BYTE_UNITS[unit + 1]}`;
  // Choose precision after rounding, so 99.97 becomes `100`, not `100.0`.
  return `${roundsBelow(value, 100) ? value.toFixed(1) : Math.round(value)} ${BYTE_UNITS[unit]}`;
}

/**
 * One decimal below 10% (where it carries information), whole numbers above: `0.4%`, `84%`.
 * Exactly zero is `0%`: nothing in use, not a measurement that happens to round to zero.
 */
export function formatPercent(percent: number | null): string {
  if (percent === null || !Number.isFinite(percent)) return PLACEHOLDER;
  if (percent === 0) return '0%';
  return `${roundsBelow(percent, 10) ? percent.toFixed(1) : Math.round(percent)}%`;
}

/** Whether `value` stays below `limit` once rounded to one decimal. */
function roundsBelow(value: number, limit: number): boolean {
  return Math.round(value * 10) / 10 < limit;
}

export function formatLoad(load: number | null): string {
  if (load === null || !Number.isFinite(load)) return PLACEHOLDER;
  return load.toFixed(2);
}

const DURATION_UNITS = [
  ['d', 86_400],
  ['h', 3_600],
  ['m', 60],
  ['s', 1],
] as const;

/**
 * The two most significant non-zero units, floored: `8d 2h`, `3h 12m`, `42s`. Precise enough to
 * read at a glance; exact values belong in a tooltip.
 */
export function formatDuration(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds) || seconds < 0) return PLACEHOLDER;
  let rest = Math.floor(seconds);
  const parts: string[] = [];
  for (const [suffix, size] of DURATION_UNITS) {
    const count = Math.floor(rest / size);
    rest -= count * size;
    if (count > 0 || parts.length > 0) parts.push(`${count}${suffix}`);
    if (parts.length === 2) break;
  }
  // Drop a trailing zero unit: `2h 0m` reads better as `2h`.
  if (parts.length === 2 && parts[1]?.startsWith('0')) parts.pop();
  return parts.length > 0 ? parts.join(' ') : '0s';
}

/** Below this, a past time reads as "just now" rather than a ticking seconds count. */
const JUST_NOW_SECONDS = 5;

/**
 * Time relative to `now` (both unix seconds), using the largest whole unit: `42s ago`, `3h ago`,
 * `in 5m`. Callers pass a ticking `now` so the text stays current between polls.
 */
export function formatRelativeTime(ts: number | null, now: number): string {
  if (!isTimestamp(ts) || !Number.isFinite(now)) return PLACEHOLDER;
  const diff = Math.round(now - ts);
  if (Math.abs(diff) < JUST_NOW_SECONDS) return 'just now';
  const magnitude = Math.abs(diff);
  const [suffix, size] = DURATION_UNITS.find(([, s]) => magnitude >= s) ?? DURATION_UNITS[3];
  const text = `${Math.floor(magnitude / size)}${suffix}`;
  return diff > 0 ? `${text} ago` : `in ${text}`;
}

/** A next run this close behind `now` is still "due now": systemd may take a moment to fire it. */
const DUE_GRACE_SECONDS = 60;

/**
 * Time until `ts` as a ticking countdown, two units: `in 3h 12m`, `in 42s`. A time just passed
 * reads `due now`; one well past reads `overdue 5m`, since a next run should never be behind.
 */
export function formatCountdown(ts: number | null, now: number): string {
  if (!isTimestamp(ts) || !Number.isFinite(now)) return PLACEHOLDER;
  const diff = Math.floor(ts - now);
  if (diff >= 1) return `in ${formatDuration(diff)}`;
  if (-diff < DUE_GRACE_SECONDS) return 'due now';
  return `overdue ${formatDuration(-diff)}`;
}

/** Absolute local date and time, for tooltips next to relative times. */
export function formatTimestamp(ts: number | null): string {
  if (!isTimestamp(ts)) return PLACEHOLDER;
  return new Date(ts * 1000).toLocaleString(undefined, {
    dateStyle: 'medium',
    timeStyle: 'medium',
  });
}

/** The backend sends `0` for "not yet" (e.g. `system.ts` before the first sample), not 1970. */
export function isTimestamp(ts: number | null): ts is number {
  return ts !== null && Number.isFinite(ts) && ts > 0;
}

/**
 * Splits a URL, address or unit name after its natural separators (`/`, `.`, `:`, `@`, `-`, `_`)
 * so the parts can be joined with `<wbr>`: long targets then wrap between path segments instead
 * of mid-word. Joining the parts gives back the input exactly.
 */
export function breakPoints(text: string): string[] {
  return text.split(/(?<=[/.:@_-])/).filter((part) => part !== '');
}
