// Turns `/metrics/query` responses into column arrays for charts. Null buckets stay null all the
// way to the chart, which draws them as gaps (ARCHITECTURE §6).

import type { MetricsQuery } from '../api/types';

export type Column = (number | null)[];

export interface SeriesColumns {
  avg: Column;
  min: Column;
  max: Column;
}

export interface ChartData {
  /** Bucket start times, ascending, unix seconds. */
  ts: number[];
  /** One entry per requested key, in request order. */
  series: SeriesColumns[];
}

/**
 * Aligns the response's series on one shared, sorted time axis. A key the response lacks, or a
 * bucket one series has and another does not, becomes null rather than being dropped, so every
 * column has the same length as `ts`.
 */
export function alignSeries(response: MetricsQuery, keys: readonly string[]): ChartData {
  const byKey = new Map(response.series.map((s) => [s.key, s]));
  const ts = [...new Set(response.series.flatMap((s) => s.points.map((p) => p[0])))].sort(
    (a, b) => a - b,
  );
  const index = new Map(ts.map((t, i) => [t, i]));

  const series = keys.map((key) => {
    const columns: SeriesColumns = {
      avg: ts.map(() => null),
      min: ts.map(() => null),
      max: ts.map(() => null),
    };
    for (const [t, avg, min, max] of byKey.get(key)?.points ?? []) {
      const i = index.get(t);
      if (i === undefined) continue;
      columns.avg[i] = avg;
      columns.min[i] = min;
      columns.max[i] = max;
    }
    return columns;
  });
  return { ts, series };
}

/** True when at least one bucket of one series has a value. */
export function hasValues(data: ChartData): boolean {
  return data.series.some((s) => s.avg.some((v) => v !== null));
}

/** uPlot's column layout: `[ts, avg₀, min₀, max₀, avg₁, min₁, max₁, …]`. */
export function toPlotColumns(data: ChartData): [number[], ...Column[]] {
  return [data.ts, ...data.series.flatMap((s) => [s.avg, s.min, s.max])];
}

/** The last non-null value of a column, e.g. for end labels and summaries. */
export function lastValue(column: Column): number | null {
  for (let i = column.length - 1; i >= 0; i -= 1) {
    const value = column[i];
    if (value !== null && value !== undefined) return value;
  }
  return null;
}

/** Smallest and largest non-null values across columns; null when there are none. */
export function extent(columns: readonly Column[]): { min: number; max: number } | null {
  let min = Infinity;
  let max = -Infinity;
  for (const column of columns) {
    for (const value of column) {
      if (value === null) continue;
      if (value < min) min = value;
      if (value > max) max = value;
    }
  }
  return min === Infinity ? null : { min, max };
}

/** Each series' average column keyed by series key, for sparklines over a glob query. */
export function averagesByKey(response: MetricsQuery): Map<string, Column> {
  const keys = response.series.map((s) => s.key);
  const aligned = alignSeries(response, keys);
  return new Map(keys.map((key, i) => [key, aligned.series[i]?.avg ?? []]));
}
