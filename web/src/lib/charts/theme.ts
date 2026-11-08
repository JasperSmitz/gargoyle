// Chart colors come from tokens.css; canvas drawing needs them as concrete strings, so they are
// read from computed style when a chart is built and re-read when the color scheme flips.

export interface ChartTheme {
  series: string[];
  surface: string;
  grid: string;
  axisText: string;
  labelText: string;
  font: string;
}

export function readChartTheme(el: Element, slots: number): ChartTheme {
  const style = getComputedStyle(el);
  const read = (name: string, fallback: string) => style.getPropertyValue(name).trim() || fallback;
  return {
    series: Array.from({ length: slots }, (_, i) => read(`--chart-series-${i + 1}`, '#2a78d6')),
    surface: read('--color-surface', '#ffffff'),
    grid: read('--chart-grid', '#e5e5e5'),
    axisText: read('--chart-axis-text', '#707070'),
    labelText: read('--color-text-muted', '#555555'),
    font: read('--font-sans', 'sans-serif'),
  };
}

/** `#rrggbb` (or `#rgb`) at `alpha`, for band fills; other color syntaxes pass through unchanged. */
export function withAlpha(color: string, alpha: number): string {
  let hex = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(color)?.[1];
  if (!hex) return color;
  if (hex.length === 3) hex = [...hex].map((c) => c + c).join('');
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16));
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/** Calls `onChange` when the OS color scheme flips; returns an unsubscribe function. */
export function watchColorScheme(onChange: () => void): () => void {
  if (typeof matchMedia !== 'function') return () => {};
  const query = matchMedia('(prefers-color-scheme: dark)');
  query.addEventListener('change', onChange);
  return () => query.removeEventListener('change', onChange);
}
