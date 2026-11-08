// uPlot configuration for TimeSeriesChart, kept out of the component so it can be read (and
// tested) as plain data. Mark specs follow the dataviz rules: 2px lines, the min/max band as a
// ~12% wash of the series hue, a recessive hairline grid, no axis ticks, a crosshair that snaps
// to the nearest bucket, and gaps (never interpolation) for null buckets.

import uPlot from 'uplot';
import type { SeriesUnit } from '../api/types';
import type { ChartSeries } from './groups';
import { formatTimeTicks } from './time';
import type { ChartTheme } from './theme';
import { withAlpha } from './theme';
import { axisIncrements, formatAxisTicks, yRange } from './units';

/** Columns per series in the plot data: avg, min, max (see `toPlotColumns`). */
const COLUMNS = 3;
/**
 * Band opacity by series count: a single band reads at 12%; two overlapping bands drop to 7% so
 * the overlap stays legible. From three series on, only the hovered series shows its band
 * (overlapping washes would turn to mud), so it can use the full 12%.
 */
export function bandAlpha(seriesCount: number): number {
  return seriesCount === 2 ? 0.07 : 0.12;
}

/** Series counts from which bands are drawn only for the hovered series. */
const FOCUS_BANDS_FROM = 3;
const LABEL_FONT_PX = 12;
/** Approximate width of one label character at 12px, for gutter sizing without a canvas. */
const CHAR_PX = 7;
const MIN_RIGHT_GUTTER = 32;
/**
 * Minimum px between time ticks: the widest label ("Nov 2026", ~60px) plus breathing room, so
 * ticks thin out on narrow charts instead of running together.
 */
const TIME_TICK_SPACE = 80;
/** Height of the time axis: one 12px label line and a small gap above it. */
const TIME_AXIS_SIZE = 28;

export interface PlotConfig {
  unit: SeriesUnit;
  series: readonly ChartSeries[];
  theme: ChartTheme;
  width: number;
  height: number;
  /** Labels at the right end of each line (used for 2–4 series alongside the legend). */
  directLabels: boolean;
  /** Called on every cursor move; `idx` is null when the pointer leaves the plot. */
  onCursor: (idx: number | null, left: number, top: number) => void;
}

/** Index in `u.series` of series `i`'s average; its min and max follow it. */
export function avgIndex(i: number): number {
  return 1 + i * COLUMNS;
}

function isAverage(seriesIdx: number): boolean {
  return seriesIdx > 0 && (seriesIdx - 1) % COLUMNS === 0;
}

function finite(value: number | null | undefined): number | null {
  return value !== null && value !== undefined && Number.isFinite(value) ? value : null;
}

export function buildPlotOptions(config: PlotConfig): uPlot.Options {
  const { unit, series, theme, directLabels } = config;
  const color = (i: number) => theme.series[(series[i]?.slot ?? 0) % theme.series.length] ?? '';
  const font = `${LABEL_FONT_PX}px ${theme.font}`;
  const longestLabel = Math.max(0, ...series.map((s) => s.label.length));
  // Without end labels the gutter still has to hold half of a centred last x tick ("Nov 2026").
  const rightGutter = Math.max(MIN_RIGHT_GUTTER, directLabels ? 12 + longestLabel * CHAR_PX : 0);

  const focusBands = series.length >= FOCUS_BANDS_FROM;
  const alpha = bandAlpha(series.length);
  let focused: number | null = null;

  const axisBase = {
    stroke: theme.axisText,
    font,
    grid: { stroke: theme.grid, width: 1 },
    ticks: { show: false },
  };

  return {
    width: config.width,
    height: config.height,
    padding: [12, rightGutter, 0, 0],
    legend: { show: false },
    cursor: {
      y: false,
      drag: { x: false, y: false, setScale: false },
      points: {
        size: (_u, si) => (isAverage(si) ? 8 : 0),
        // Min/max helper series get no dot at all: a zero-size point with a ring still paints.
        width: (_u, si) => (isAverage(si) ? 2 : 0),
        stroke: theme.surface,
        fill: (_u, si) => color(Math.floor((si - 1) / COLUMNS)),
      },
    },
    scales: {
      x: { time: true },
      y: { range: (_u, min, max) => yRange(unit, finite(min), finite(max)) },
    },
    axes: [
      {
        ...axisBase,
        space: TIME_TICK_SPACE,
        size: TIME_AXIS_SIZE,
        values: (_u, splits, _axis, _space, incr) => formatTimeTicks(splits, incr),
      },
      {
        ...axisBase,
        incrs: axisIncrements(unit),
        values: (_u, splits) => formatAxisTicks(unit, splits),
        size: (_u, values) =>
          values ? 16 + Math.max(0, ...values.map((v) => v.length)) * CHAR_PX : 48,
      },
    ],
    series: [
      {},
      ...series.flatMap((s, i) => [
        { label: s.label, stroke: color(i), width: 2, points: { show: false } },
        { label: `${s.label} min`, stroke: 'transparent', width: 0, points: { show: false } },
        { label: `${s.label} max`, stroke: 'transparent', width: 0, points: { show: false } },
      ]),
    ],
    bands: series.map((_s, i) => ({
      series: [avgIndex(i) + 2, avgIndex(i) + 1] as [number, number],
      // A function: uPlot evaluates it on every draw, so focus changes need only a redraw.
      fill: () => (!focusBands || focused === i ? withAlpha(color(i), alpha) : 'transparent'),
    })),
    hooks: {
      setCursor: [
        (u) => {
          const { idx, left, top } = u.cursor;
          config.onCursor(idx ?? null, left ?? 0, top ?? 0);
          if (!focusBands) return;
          const next = idx === null || idx === undefined ? null : nearestSeries(u, idx, top ?? 0);
          if (next !== focused) {
            focused = next;
            u.redraw(false);
          }
        },
      ],
      draw: directLabels ? [(u) => drawEndLabels(u, series, theme)] : [],
    },
  };
}

/**
 * Series names at the right end of each line, in label ink (never the series color). When two
 * labels would collide none are drawn: nudging detaches labels from their lines, and the legend
 * and tooltip still identify every series.
 */
function drawEndLabels(u: uPlot, series: readonly ChartSeries[], theme: ChartTheme): void {
  const ratio = uPlot.pxRatio;
  const labels: { text: string; y: number }[] = [];
  series.forEach((s, i) => {
    const column = u.data[avgIndex(i)] ?? [];
    for (let j = column.length - 1; j >= 0; j -= 1) {
      const value = column[j];
      if (value === null || value === undefined) continue;
      labels.push({ text: s.label, y: u.valToPos(value, 'y', true) });
      return;
    }
  });
  labels.sort((a, b) => a.y - b.y);
  const minGap = (LABEL_FONT_PX + 2) * ratio;
  if (labels.some((l, i) => i > 0 && l.y - (labels[i - 1]?.y ?? -Infinity) < minGap)) return;

  const { ctx } = u;
  ctx.save();
  ctx.font = `${LABEL_FONT_PX * ratio}px ${theme.font}`;
  ctx.fillStyle = theme.labelText;
  // Set explicitly: the context keeps whatever alignment uPlot last drew its axis text with.
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  const x = u.bbox.left + u.bbox.width + 6 * ratio;
  for (const label of labels) ctx.fillText(label.text, x, label.y);
  ctx.restore();
}

/**
 * Left edge for a tooltip `width` px wide next to a crosshair at `at`, inside a chart `bounds` px
 * wide: right of the crosshair if it fits, else left of it, else pinned to stay inside.
 */
export function placeTooltip(at: number, width: number, bounds: number, gap = 12): number {
  let x = at + gap;
  if (x + width > bounds) x = at - gap - width;
  if (x < 0) x = Math.max(0, bounds - width);
  return x;
}

/**
 * The series whose average at bucket `idx` is drawn closest to the pointer's height `top` (CSS px
 * within the plot), or null when no series has a value there.
 */
export function nearestSeries(u: uPlot, idx: number, top: number): number | null {
  let best: number | null = null;
  let bestDistance = Infinity;
  for (let i = 0; avgIndex(i) < u.data.length; i += 1) {
    const value = u.data[avgIndex(i)]?.[idx];
    if (value === null || value === undefined) continue;
    const distance = Math.abs(u.valToPos(value, 'y') - top);
    if (distance < bestDistance) {
      bestDistance = distance;
      best = i;
    }
  }
  return best;
}
