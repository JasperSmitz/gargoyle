// Geometry for overview sparklines: a tiny trend line with no axes, gaps for null buckets.

import type { Column } from './data';
import { extent } from './data';

export interface SparklineGeometry {
  /** SVG path in a `0 0 width height` box; one `M` per run of non-null values. */
  path: string;
  /** The last drawn point, for the end dot; null when nothing is drawn. */
  end: { x: number; y: number } | null;
}

/** `minSpan` is the smallest vertical extent, so idle jitter does not fill the whole height. */
export function sparklinePath(
  values: Column,
  width: number,
  height: number,
  minSpan: number,
): SparklineGeometry {
  const span = extent([values]);
  if (!span || values.length === 0) return { path: '', end: null };
  // Centre the data in at least `minSpan`, never dipping below zero for non-negative metrics.
  const pad = Math.max(0, minSpan - (span.max - span.min)) / 2;
  const low = Math.max(span.min >= 0 ? 0 : -Infinity, span.min - pad);
  const high = Math.max(low + minSpan, span.max + pad);
  const step = values.length > 1 ? width / (values.length - 1) : 0;
  const y = (v: number) => height - ((v - low) / (high - low)) * height;

  let path = '';
  let end: SparklineGeometry['end'] = null;
  let penDown = false;
  values.forEach((value, i) => {
    if (value === null) {
      penDown = false;
      return;
    }
    const point = { x: round(i * step), y: round(y(value)) };
    path += `${penDown ? 'L' : 'M'}${point.x} ${point.y}`;
    penDown = true;
    end = point;
  });
  return { path, end };
}

function round(value: number): number {
  return Math.round(value * 100) / 100;
}
