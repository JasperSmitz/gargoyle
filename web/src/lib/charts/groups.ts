// Which charts the System page shows, derived from the series the backend has (`/metrics`).
// A chart group is one `/metrics/query` request; it may render as several charts (one per
// network interface). Absent metrics produce no chart at all: no temperature sensors, no
// temperature chart.

import type { SeriesInfo, SeriesUnit, UnixSeconds } from '../api/types';
import { type Range, rangeWindow } from './range';

/** Categorical slots defined in tokens.css (`--chart-series-1` … `--chart-series-8`). */
export const SERIES_SLOTS = 8;

export interface ChartSeries {
  key: string;
  label: string;
  /** Zero-based categorical slot. Fixed per entity, never by rank, so colors do not shuffle. */
  slot: number;
}

export interface ChartSpec {
  id: string;
  title: string;
  unit: SeriesUnit;
  series: ChartSeries[];
}

export interface ChartGroup {
  id: string;
  /** Keys for the query; labelled metrics use a `*` glob so the request stays within 12 keys. */
  queryKeys: string[];
  charts: ChartSpec[];
}

interface FixedSeries {
  key: string;
  label: string;
}

/** Charts over a fixed set of unlabelled series; the slot is the position in this list. */
function fixedChart(
  available: ReadonlySet<string>,
  id: string,
  title: string,
  unit: SeriesUnit,
  members: FixedSeries[],
): ChartGroup | null {
  const series = members.map((m, slot) => ({ ...m, slot })).filter((m) => available.has(m.key));
  if (series.length === 0) return null;
  return { id, queryKeys: series.map((s) => s.key), charts: [{ id, title, unit, series }] };
}

/**
 * One series per label of `metric`, sorted by label; more than 8 split into small multiples.
 * Slots follow the sorted label, which is stable for a host's fixed mounts and sensors; a new
 * label sorting before existing ones would shift their colors, accepted for that rare case.
 */
function labelledChart(
  all: readonly SeriesInfo[],
  metric: string,
  id: string,
  title: string,
  unit: SeriesUnit,
): ChartGroup | null {
  const labels = all
    .filter((s) => s.metric === metric && s.label !== null)
    .map((s) => ({ key: s.key, label: s.label ?? '' }))
    .sort((a, b) => a.label.localeCompare(b.label));
  if (labels.length === 0) return null;

  const chunks: ChartSpec[] = [];
  for (let start = 0; start < labels.length; start += SERIES_SLOTS) {
    const part = labels.slice(start, start + SERIES_SLOTS);
    const count = Math.ceil(labels.length / SERIES_SLOTS);
    const n = chunks.length + 1;
    chunks.push({
      id: count > 1 ? `${id}-${n}` : id,
      title: count > 1 ? `${title} (${n} of ${count})` : title,
      unit,
      series: part.map((s, slot) => ({ ...s, slot })),
    });
  }
  return { id, queryKeys: [`${metric}:*`], charts: chunks };
}

/** Network: one chart per interface with received and sent rates on the same axis. */
function networkCharts(all: readonly SeriesInfo[]): ChartGroup | null {
  const ifaces = [
    ...new Set(
      all
        .filter((s) => (s.metric === 'net.rx' || s.metric === 'net.tx') && s.label !== null)
        .map((s) => s.label ?? ''),
    ),
  ].sort((a, b) => a.localeCompare(b));
  if (ifaces.length === 0) return null;

  const keys = new Set(all.map((s) => s.key));
  const charts = ifaces.map((iface) => ({
    id: `net-${iface}`,
    title: `Network ${iface}`,
    unit: 'bytes_per_s' as const,
    series: [
      { key: `net.rx:${iface}`, label: 'Received', slot: 0 },
      { key: `net.tx:${iface}`, label: 'Sent', slot: 1 },
    ].filter((s) => keys.has(s.key)),
  }));
  return { id: 'net', queryKeys: ['net.rx:*', 'net.tx:*'], charts };
}

export function buildChartGroups(all: readonly SeriesInfo[]): ChartGroup[] {
  const keys = new Set(all.map((s) => s.key));
  const groups = [
    fixedChart(keys, 'cpu', 'CPU', 'percent', [
      { key: 'cpu.usage', label: 'Usage' },
      { key: 'cpu.iowait', label: 'I/O wait' },
    ]),
    fixedChart(keys, 'load', 'Load average', 'load', [
      { key: 'load.1', label: '1 min' },
      { key: 'load.5', label: '5 min' },
      { key: 'load.15', label: '15 min' },
    ]),
    fixedChart(keys, 'memory', 'Memory and swap', 'percent', [
      { key: 'mem.used', label: 'Memory' },
      { key: 'swap.used', label: 'Swap' },
    ]),
    labelledChart(all, 'disk.used', 'disk', 'Disk usage', 'percent'),
    networkCharts(all),
    labelledChart(all, 'temp', 'temps', 'Temperatures', 'celsius'),
  ];
  return groups.filter((g): g is ChartGroup => g !== null);
}

/** Buckets per chart; the backend's default, enough for a full-width chart at 1–4 px each. */
export const CHART_POINTS = 300;

export interface ChartQuery {
  keys: string[];
  from: UnixSeconds;
  to: UnixSeconds;
  points: number;
}

export function buildQuery(
  keys: readonly string[],
  range: Range,
  now: number,
  points = CHART_POINTS,
): ChartQuery {
  return { keys: [...keys], ...rangeWindow(range, now), points };
}
