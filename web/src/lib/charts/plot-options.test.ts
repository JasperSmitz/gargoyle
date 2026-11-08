import { describe, expect, it, vi } from 'vitest';
import uPlot from 'uplot';
import { avgIndex, buildPlotOptions, placeTooltip } from './plot-options';
import type { ChartTheme } from './theme';
import { withAlpha } from './theme';

const theme: ChartTheme = {
  series: ['#2a78d6', '#eb6834', '#1baf7a'],
  surface: '#fbfaf8',
  grid: '#ebe8e2',
  axisText: '#736d63',
  labelText: '#6a645b',
  font: 'system-ui',
};

function bandFill(opts: uPlot.Options, i: number): unknown {
  const fill = opts.bands?.[i]?.fill;
  return typeof fill === 'function' ? fill({} as uPlot, i, '') : fill;
}

function options(slots: number[], label = 'S') {
  return buildPlotOptions({
    unit: 'percent',
    series: slots.map((slot, i) => ({ key: `k${i}`, label: `${label}${i}`, slot })),
    theme,
    width: 400,
    height: 200,
    directLabels: slots.length >= 2,
    onCursor: vi.fn(),
  });
}

describe('buildPlotOptions', () => {
  it('draws 2px lines in the slot color and hides the min/max helper series', () => {
    const opts = options([1, 0]);
    expect(opts.series).toHaveLength(7);
    expect(opts.series[avgIndex(0)]).toMatchObject({ stroke: '#eb6834', width: 2 });
    expect(opts.series[avgIndex(1)]).toMatchObject({ stroke: '#2a78d6', width: 2 });
    expect(opts.series[avgIndex(0) + 1]).toMatchObject({ stroke: 'transparent', width: 0 });
  });

  it('fills a single min/max band with a light wash of its series color', () => {
    const opts = options([2]);
    expect(opts.bands?.[0]?.series).toEqual([3, 2]);
    expect(bandFill(opts, 0)).toBe(withAlpha('#1baf7a', 0.12));
  });

  it('lowers band opacity for two series so the overlap stays readable', () => {
    const opts = options([0, 1]);
    expect(bandFill(opts, 0)).toBe(withAlpha('#2a78d6', 0.07));
    expect(bandFill(opts, 1)).toBe(withAlpha('#eb6834', 0.07));
  });

  it('with 3+ series draws only the hovered series band, none when not hovering', () => {
    const opts = options([0, 1, 2]);
    expect([0, 1, 2].map((i) => bandFill(opts, i))).toEqual([
      'transparent',
      'transparent',
      'transparent',
    ]);
    // Bucket 1: series values 10, 50, 90; the pointer is drawn nearest to 50.
    const redraw = vi.fn();
    const u = {
      cursor: { idx: 1, left: 0, top: 150 },
      data: [
        [1, 2],
        ...[10, 50, 90].flatMap((v) => [
          [0, v],
          [0, v],
          [0, v],
        ]),
      ],
      valToPos: (v: number) => 200 - v,
      redraw,
    } as unknown as uPlot;
    for (const hook of opts.hooks?.setCursor ?? []) hook?.(u);
    expect(redraw).toHaveBeenCalledOnce();
    expect([0, 1, 2].map((i) => bandFill(opts, i))).toEqual([
      'transparent',
      withAlpha('#eb6834', 0.12),
      'transparent',
    ]);

    (u.cursor as { idx: number | null }).idx = null;
    for (const hook of opts.hooks?.setCursor ?? []) hook?.(u);
    expect(bandFill(opts, 1)).toBe('transparent');
  });

  it('keeps one y axis, never spans gaps, and disables drag-zoom', () => {
    const opts = options([0, 1]);
    expect(opts.axes).toHaveLength(2);
    expect(opts.series.some((s) => s.spanGaps)).toBe(false);
    expect(opts.cursor?.drag).toMatchObject({ x: false, y: false, setScale: false });
    expect(opts.legend?.show).toBe(false);
  });

  it('reserves a right gutter for direct labels only when they are drawn', () => {
    expect(options([0, 1], 'Received').padding?.[1]).toBeGreaterThan(
      options([0], 'Received').padding?.[1] as number,
    );
  });
});

describe('withAlpha', () => {
  it('converts hex colors and passes others through', () => {
    expect(withAlpha('#ff0080', 0.5)).toBe('rgba(255, 0, 128, 0.5)');
    expect(withAlpha('#f08', 0.1)).toBe('rgba(255, 0, 136, 0.1)');
    expect(withAlpha('rgb(1 2 3)', 0.1)).toBe('rgb(1 2 3)');
  });
});

describe('end labels', () => {
  // uPlot reads devicePixelRatio in a browser; node leaves it unset.
  uPlot.pxRatio = 1;

  function drawWith(lastValues: (number | null)[][]) {
    const opts = options(lastValues.map((_, i) => i));
    const texts: { text: string; x: number; align: string }[] = [];
    const ctx = {
      textAlign: 'right',
      save() {},
      restore() {},
      fillText(text: string, x: number) {
        texts.push({ text, x, align: ctx.textAlign });
      },
    };
    const u = {
      ctx,
      bbox: { left: 50, width: 300 },
      data: [[1, 2], ...lastValues.flatMap((v) => [v, v, v])],
      valToPos: (v: number) => 200 - v,
    } as unknown as uPlot;
    for (const hook of opts.hooks?.draw ?? []) hook?.(u);
    return texts;
  }

  it('draws left-aligned in the right gutter, past the plot area', () => {
    const texts = drawWith([
      [10, 20],
      [60, 80],
    ]);
    expect(texts.map((t) => t.text)).toEqual(['S1', 'S0']);
    for (const t of texts) {
      expect(t.align).toBe('left');
      expect(t.x).toBeGreaterThan(350);
    }
  });

  it('draws none when two labels would collide', () => {
    expect(
      drawWith([
        [10, 20],
        [10, 25],
      ]),
    ).toEqual([]);
  });
});

describe('x axis', () => {
  it('spaces time ticks for their label width and keeps the axis band short', () => {
    const [x] = options([0]).axes ?? [];
    expect(x?.space).toBeGreaterThanOrEqual(64);
    expect(x?.size).toBe(28);
  });

  it('gives cursor dots only to average series', () => {
    const points = options([0]).cursor?.points;
    const width = points?.width as (u: uPlot, si: number, size: number) => number;
    expect(typeof width).toBe('function');
    expect(width({} as uPlot, 1, 8)).toBe(2);
    expect(width({} as uPlot, 2, 0)).toBe(0);
  });
});

describe('placeTooltip', () => {
  it('sits right of the crosshair when it fits', () => {
    expect(placeTooltip(100, 150, 400)).toBe(112);
  });

  it('flips left when the right side would overflow', () => {
    expect(placeTooltip(300, 150, 400)).toBe(138);
  });

  it('pins inside the chart when neither side fits (narrow phones)', () => {
    expect(placeTooltip(130, 190, 280)).toBe(90);
    expect(placeTooltip(10, 300, 280)).toBe(0);
  });
});
