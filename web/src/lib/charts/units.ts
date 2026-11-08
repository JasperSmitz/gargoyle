// Unit-aware formatting and scales for charts. Units are the contract's series units
// (ARCHITECTURE §5): percent 0–100, bytes, bytes_per_s, celsius, load and ms.

import type { SeriesUnit } from '../api/types';
import { PLACEHOLDER, formatBytes, formatLoad, formatPercent } from '../format';

/** A single value for tooltips, tables and summaries: `12%`, `117.6 KiB/s`, `48.0 °C`. */
export function formatValue(unit: SeriesUnit, value: number | null): string {
  if (value === null || !Number.isFinite(value)) return PLACEHOLDER;
  switch (unit) {
    case 'percent':
      return formatPercent(value);
    case 'bytes':
      return formatBytes(value);
    case 'bytes_per_s':
      return value < 0 ? PLACEHOLDER : `${formatBytes(value)}/s`;
    case 'celsius':
      return `${value.toFixed(1)} °C`;
    case 'load':
      return formatLoad(value);
    case 'ms':
      return formatMs(value);
  }
}

function formatMs(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(2)} s`;
  return `${ms < 10 ? ms.toFixed(1) : Math.round(ms)} ms`;
}

const IEC = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'] as const;

/**
 * Labels for one axis's ticks. One style per axis: byte axes pick a single IEC unit from the
 * largest tick, so a column reads `0 / 0.5 / 1 / 1.5 GiB` rather than mixing MiB and GiB.
 */
export function formatAxisTicks(unit: SeriesUnit, ticks: readonly number[]): string[] {
  switch (unit) {
    case 'percent':
      return ticks.map((t) => `${trim(t)}%`);
    case 'bytes':
    case 'bytes_per_s': {
      const largest = Math.max(0, ...ticks.map(Math.abs));
      let power = 0;
      while (largest >= 1024 ** (power + 1) && power < IEC.length - 1) power += 1;
      const suffix = IEC[power] + (unit === 'bytes_per_s' ? '/s' : '');
      return ticks.map((t) => `${trim(t / 1024 ** power)} ${suffix}`);
    }
    case 'celsius':
      return ticks.map((t) => `${trim(t)} °C`);
    case 'load':
      return ticks.map((t) => trim(t));
    case 'ms': {
      const seconds = Math.max(0, ...ticks) >= 1000;
      return ticks.map((t) => (seconds ? `${trim(t / 1000)} s` : `${trim(t)} ms`));
    }
  }
}

/** Drops float noise from tick values: 0.30000000000000004 → `0.3`. */
function trim(value: number): string {
  return String(Number(value.toFixed(2)));
}

/**
 * Tick increments for byte axes, so ticks land on round binary sizes (500 MiB, 1 GiB, 2 GiB) instead of
 * decimal ones that would print as `953.7 MiB`. Other units use uPlot's decimal increments.
 */
export function axisIncrements(unit: SeriesUnit): number[] | undefined {
  if (unit !== 'bytes' && unit !== 'bytes_per_s') return undefined;
  const steps = [1, 2, 5, 10, 20, 50, 100, 200, 500];
  return IEC.flatMap((_, power) => steps.map((s) => s * 1024 ** power));
}

/** Smallest y extent per unit, so a flat idle series does not blow noise up to full height. */
const MIN_EXTENT: Record<Exclude<SeriesUnit, 'percent' | 'celsius'>, number> = {
  bytes: 1024 * 1024,
  bytes_per_s: 1024,
  load: 1,
  ms: 10,
};

/**
 * The y scale for a chart. Percent is always the full 0–100. Quantities that cannot go negative
 * start at zero so heights compare honestly; temperatures float in 10-degree steps around the data.
 * `min`/`max` are null when the visible data has no values.
 */
export function yRange(unit: SeriesUnit, min: number | null, max: number | null): [number, number] {
  if (unit === 'percent') return [0, 100];
  if (unit === 'celsius') {
    if (min === null || max === null) return [0, 100];
    return [Math.max(0, Math.floor((min - 5) / 10) * 10), Math.ceil((max + 5) / 10) * 10];
  }
  const top = max === null ? 0 : max * 1.1;
  return [0, Math.max(top, MIN_EXTENT[unit])];
}
