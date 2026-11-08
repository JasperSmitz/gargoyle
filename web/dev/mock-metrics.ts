// Generated `/metrics` and `/metrics/query` responses for the dev mock. Values are a pure function
// of (series key, time) (minecraft's restarts also of "now", see MINECRAFT), so every range and every refresh agrees with itself: zooming from 1y to
// 1h shows the same history, just finer. A short outage every 3.5 days leaves null buckets so
// gaps can be seen, and resolution/step follow ARCHITECTURE §5 (raw for ≤ 6h within 48h).

import { fixtures } from '../src/lib/api/fixtures';
import type { MetricPoint, MetricsQuery, SeriesInfo, SeriesUnit } from '../src/lib/api/types';
import { LATENCY_CHECKS, latencyMissing, latencySample } from './mock-checks';

const HOST = 'debian-box';
const RAW_INTERVAL = 10;
const ROLLUP = 300;
const RAW_RETENTION = 48 * 3600;
const OUTAGE_PERIOD = 3.5 * 86_400;
const OUTAGE_LENGTH = 45 * 60;
/** Samples per bucket: at least 6, and one per 10 minutes so long steps do not alias the daily cycle. */
const MIN_SAMPLES = 6;
const SAMPLE_SPACING = 600;

const SERIES: [key: string, unit: SeriesUnit][] = [
  ['cpu.usage', 'percent'],
  ['cpu.iowait', 'percent'],
  ['load.1', 'load'],
  ['load.5', 'load'],
  ['load.15', 'load'],
  ['mem.used', 'percent'],
  ['swap.used', 'percent'],
  ['disk.used:/', 'percent'],
  ['disk.used:/srv', 'percent'],
  ['net.rx:enp1s0', 'bytes_per_s'],
  ['net.tx:enp1s0', 'bytes_per_s'],
  ['temp:coretemp/Package id 0', 'celsius'],
  ...LATENCY_CHECKS.map((name): [string, SeriesUnit] => [`check.latency:${name}`, 'ms']),
  // Only the running service: minecraft.service has never reported memory, like a real unit
  // that failed before the first sample, which the service page shows as "no data".
  ['service.memory:navidrome.service', 'bytes'],
  ['service.restarts:navidrome.service', 'count'],
  ['service.restarts:minecraft.service', 'count'],
];

export function seriesInfo(): SeriesInfo[] {
  return SERIES.map(([key, unit]) => {
    const colon = key.indexOf(':');
    return {
      key,
      metric: colon === -1 ? key : key.slice(0, colon),
      label: colon === -1 ? null : key.slice(colon + 1),
      unit,
    };
  });
}

export class QueryError extends Error {
  constructor(
    message: string,
    /** The backend's error code: `unknown_series` for a key it never stored. */
    readonly code: 'bad_request' | 'unknown_series' = 'bad_request',
  ) {
    super(message);
  }
}

/** Answers `/metrics/query` for `params` as the backend would, at server time `now`. */
export function query(params: URLSearchParams, now: number): MetricsQuery {
  const requested = params.getAll('key');
  if (requested.length === 0) throw new QueryError('at least one key is required');
  if (requested.length > 12) throw new QueryError('at most 12 keys');
  const keys = requested.flatMap((key) => {
    if (key.endsWith('*')) return SERIES.filter(([k]) => k.startsWith(key.slice(0, -1)));
    const match = SERIES.find(([k]) => k === key);
    if (!match) throw new QueryError(`unknown series key: ${key}`, 'unknown_series');
    return [match];
  });

  const to = intParam(params, 'to') ?? Math.floor(now);
  const from = intParam(params, 'from') ?? to - 3600;
  if (from >= to) throw new QueryError('from must be before to');
  const points = Math.min(1000, Math.max(10, intParam(params, 'points') ?? 300));
  const raw = to - from <= 6 * 3600 && from >= now - RAW_RETENTION;
  const base = raw ? RAW_INTERVAL : ROLLUP;
  const step = Math.max(base, Math.ceil((to - from) / points / base) * base);

  const first = Math.floor(from / step) * step;
  const buckets: number[] = [];
  for (let t = first; t <= to && buckets.length < points; t += step) buckets.push(t);

  return {
    host: HOST,
    from,
    to,
    step,
    resolution: raw ? 'raw' : '5m',
    series: [...new Map(keys).entries()].map(([key, unit]) => ({
      key,
      unit,
      points: buckets.map((t) => bucket(key, t, step, now)),
    })),
  };
}

function intParam(params: URLSearchParams, name: string): number | null {
  const value = params.get(name);
  if (value === null || !/^-?\d+$/.test(value)) return null;
  return Number(value);
}

function bucket(key: string, start: number, step: number, now: number): MetricPoint {
  const values: number[] = [];
  const samples = Math.max(MIN_SAMPLES, Math.ceil(step / SAMPLE_SPACING));
  for (let i = 0; i < samples; i += 1) {
    const t = start + Math.floor((i * step) / samples);
    if (t > now || inOutage(t) || failedProbe(key, t, now)) continue;
    values.push(sample(key, t, now));
  }
  if (values.length === 0) return [start, null, null, null];
  const avg = values.reduce((a, b) => a + b, 0) / values.length;
  return [start, round(avg), round(Math.min(...values)), round(Math.max(...values))];
}

function failedProbe(key: string, t: number, now: number): boolean {
  const name = key.startsWith('check.latency:') ? key.slice('check.latency:'.length) : null;
  return name !== null && latencyMissing(name, t, now);
}

function inOutage(t: number): boolean {
  return (t + 40_000) % OUTAGE_PERIOD < OUTAGE_LENGTH;
}

const DAY = 86_400;

/**
 * The overview's minecraft.service failed `ago` seconds before "now" after `restarts` restarts
 * (mock-api keeps that offset when it shifts the example to the current time). Its restart
 * series is built from the same numbers so the chart ends where the summary's count is.
 */
const MINECRAFT = (() => {
  const overview = fixtures['overview.json'];
  const service = overview.services.find((s) => s.unit === 'minecraft.service');
  return { ago: overview.ts - (service?.since ?? overview.ts), restarts: service?.restarts ?? 0 };
})();
const RESTART_SPACING = 20 * 60;

/** NRestarts at `t`: one restart every 20 minutes before the failure, then the final count. */
function minecraftRestarts(t: number, now: number): number {
  const failedAt = now - MINECRAFT.ago;
  const done = Math.floor(
    (t - (failedAt - MINECRAFT.restarts * RESTART_SPACING)) / RESTART_SPACING,
  );
  return Math.min(MINECRAFT.restarts, Math.max(0, done));
}

/**
 * The metric at time `t`: daily rhythm, slower drifts and per-sample jitter. Only the minecraft
 * restart series depends on `now`, because the failure it leads up to is anchored to now.
 */
function sample(key: string, t: number, now: number): number {
  const daily = Math.sin((2 * Math.PI * (t - 6 * 3600)) / DAY);
  const jitter = noise(key, Math.floor(t / RAW_INTERVAL));
  const cpu = clamp(16 + 9 * daily + 5 * Math.sin(t / 3700) + 10 * jitter ** 3, 0.5, 100);
  switch (key) {
    case 'cpu.usage':
      return cpu;
    case 'cpu.iowait':
      return clamp(0.4 + 2.5 * jitter ** 6 + 0.3 * daily, 0, 100);
    case 'load.1':
      return (cpu / 100) * 4 * (0.8 + 0.4 * jitter);
    case 'load.5':
      return (cpu / 100) * 4 * (0.9 + 0.1 * jitter);
    case 'load.15':
      return ((16 + 9 * daily) / 100) * 4;
    case 'mem.used':
      return 25 + 3 * Math.sin(t / 50_000) + 1.5 * daily + jitter;
    case 'swap.used':
      return 2 + 1.5 * Math.sin(t / 400_000) ** 2;
    case 'disk.used:/':
      // Slow growth with a clean-up every 30 days.
      return 78 + 8 * (((t / (30 * DAY)) % 1) + 0.02 * jitter);
    case 'disk.used:/srv':
      return 55 + 10 * ((t / (365 * DAY)) % 1);
    case 'net.rx:enp1s0':
      return 1024 * (60 + 80 * (daily + 1) + 900 * jitter ** 12);
    case 'net.tx:enp1s0':
      return 1024 * (5 + 6 * (daily + 1) + 300 * jitter ** 16);
    case 'temp:coretemp/Package id 0':
      return 38 + cpu * 0.35 + 2 * jitter;
    case 'service.memory:navidrome.service':
      // A slow leak that a weekly restart resets, plus activity in the evening.
      return 1024 ** 2 * (120 + 60 * ((t / (7 * DAY)) % 1) + 25 * (daily + 1) + 6 * jitter);
    case 'service.restarts:navidrome.service':
      // Matches the example's `restarts: 0`.
      return 0;
    case 'service.restarts:minecraft.service':
      return minecraftRestarts(t, now);
    default:
      if (key.startsWith('check.latency:')) {
        return latencySample(key.slice('check.latency:'.length), jitter, daily);
      }
      return 0;
  }
}

/** Deterministic value in [0, 1) for a key and sample index. */
function noise(key: string, n: number): number {
  let h = 2166136261 ^ n;
  for (let i = 0; i < key.length; i += 1) h = Math.imul(h ^ key.charCodeAt(i), 16777619);
  h = Math.imul(h ^ (h >>> 15), 2246822507);
  h = Math.imul(h ^ (h >>> 13), 3266489909);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
}

function clamp(value: number, low: number, high: number): number {
  return Math.min(high, Math.max(low, value));
}

function round(value: number): number {
  return Math.round(value * 100) / 100;
}
