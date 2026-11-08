<script lang="ts">
  import uPlot from 'uplot';
  import 'uplot/dist/uPlot.min.css';
  import { untrack } from 'svelte';
  import type { SeriesUnit } from '../../lib/api/types';
  import {
    type ChartData,
    extent,
    hasValues,
    lastValue,
    toPlotColumns,
  } from '../../lib/charts/data';
  import { SERIES_SLOTS, type ChartSeries } from '../../lib/charts/groups';
  import { buildPlotOptions, placeTooltip } from '../../lib/charts/plot-options';
  import { readChartTheme, watchColorScheme } from '../../lib/charts/theme';
  import { formatChartTime } from '../../lib/charts/time';
  import { formatValue } from '../../lib/charts/units';
  import SkeletonBlock from '../skeleton/SkeletonBlock.svelte';

  let {
    title,
    unit,
    series,
    data,
    loading = false,
    error = null,
    rangeDescription,
    height = 200,
  }: {
    title: string;
    unit: SeriesUnit;
    series: ChartSeries[];
    /** Null until the first response; columns follow `series` order. */
    data: ChartData | null;
    /** A request is in flight; existing data stays visible, dimmed. */
    loading?: boolean;
    /** The latest request's failure, shown over the last good data if there is any. */
    error?: string | null;
    /** Spoken range, e.g. "last 24 hours". */
    rangeDescription: string;
    height?: number;
  } = $props();

  const id = $props.id();
  let plotEl = $state<HTMLDivElement>();
  let schemeVersion = $state(0);
  let showTable = $state(false);
  let cursor = $state<{ idx: number; left: number; top: number } | null>(null);
  let tipWidth = $state(0);
  /** Below this plot width the end-label gutter costs too much; the legend identifies series. */
  const DIRECT_LABEL_MIN_WIDTH = 420;
  /** Bumped when a resize crosses that width, so the plot is rebuilt with or without labels. */
  let relayout = $state(0);

  const drawable = $derived(data !== null && hasValues(data));
  const columns = $derived(data ? toPlotColumns(data) : null);
  // Parents rebuild `series` arrays on every poll; only a real change should rebuild the plot.
  const shape = $derived(
    `${unit}|${height}|` + series.map((s) => `${s.key}:${s.slot}:${s.label}`).join('|'),
  );
  let current: uPlot | undefined;

  $effect(() => watchColorScheme(() => (schemeVersion += 1)));

  // Rebuild when the chart's shape or the color scheme changes; plain data updates use setData.
  $effect(() => {
    const el = plotEl;
    if (!el || !drawable) return;
    void schemeVersion;
    void shape;
    void relayout;
    const isWide = (width: number) => width === 0 || width >= DIRECT_LABEL_MIN_WIDTH;
    const builtWide = isWide(el.clientWidth);
    const plot = untrack(
      () =>
        new uPlot(
          buildPlotOptions({
            unit,
            series,
            theme: readChartTheme(el, SERIES_SLOTS),
            width: el.clientWidth || 300,
            height,
            directLabels: builtWide && series.length >= 2 && series.length <= 4,
            onCursor: (idx, left, top) => (cursor = idx === null ? null : { idx, left, top }),
          }),
          columns as uPlot.AlignedData,
          el,
        ),
    );
    const resize = new ResizeObserver(() => {
      if (el.clientWidth === 0) return;
      plot.setSize({ width: el.clientWidth, height: plot.height });
      if (isWide(el.clientWidth) !== builtWide) relayout += 1;
    });
    resize.observe(el);
    current = plot;
    return () => {
      resize.disconnect();
      plot.destroy();
      current = undefined;
      cursor = null;
    };
  });

  $effect(() => {
    if (columns && current) current.setData(columns as uPlot.AlignedData);
  });

  /** The newest bucket: times on its day print without a date. */
  const reference = $derived(data?.ts.at(-1) ?? 0);

  /** `min – max` when the bucket has a spread, else null (raw buckets have min = max). */
  function spreadText(i: number, idx: number): string | null {
    const min = data?.series[i]?.min[idx] ?? null;
    const max = data?.series[i]?.max[idx] ?? null;
    if (min === null || max === null || max - min <= 0) return null;
    return `${formatValue(unit, min)} – ${formatValue(unit, max)}`;
  }

  const tooltip = $derived.by(() => {
    if (!cursor || !data || !current || !plotEl) return null;
    const { idx, left } = cursor;
    const ts = data.ts[idx];
    if (ts === undefined) return null;
    const rows = series.map((s, i) => ({
      ...s,
      value: formatValue(unit, data.series[i]?.avg[idx] ?? null),
      range: spreadText(i, idx),
    }));
    // Right of the crosshair if it fits, else left of it, else pinned inside the chart: the
    // tooltip must never widen the page (no horizontal scroll on phones).
    const x = placeTooltip(current.over.offsetLeft + left, tipWidth, plotEl.clientWidth);
    return { time: formatChartTime(ts, reference), rows, x, y: current.over.offsetTop + 8 };
  });

  const summary = $derived.by(() => {
    if (!data) return `${title}, ${rangeDescription}: loading`;
    const parts = series.map((s, i) => {
      const cols = data.series[i];
      if (!cols) return `${s.label}: no data`;
      const latest = lastValue(cols.avg);
      const span = extent([cols.min, cols.max, cols.avg]);
      if (latest === null || !span) return `${s.label}: no data`;
      return (
        `${s.label}: latest ${formatValue(unit, latest)}, ` +
        `range ${formatValue(unit, span.min)} to ${formatValue(unit, span.max)}`
      );
    });
    return `${title}, ${rangeDescription}. ${parts.join('. ')}.`;
  });
</script>

<figure class="chart" aria-labelledby="{id}-title" aria-busy={loading}>
  <figcaption class="head">
    <span class="title" id="{id}-title">{title}</span>
    {#if series.length >= 2}
      <ul class="legend" aria-label="Legend">
        {#each series as s (s.key)}
          <li><span class="key" style:--key="var(--chart-series-{s.slot + 1})"></span>{s.label}</li>
        {/each}
      </ul>
    {/if}
    {#if data && drawable}
      <button
        type="button"
        class="toggle"
        aria-expanded={showTable}
        aria-controls="{id}-table"
        onclick={() => (showTable = !showTable)}
      >
        {showTable ? 'Hide table' : 'Show table'}
      </button>
    {/if}
  </figcaption>

  <div class="frame" style:height="{height}px">
    {#if drawable}
      <div
        class="plot"
        class:refreshing={loading}
        bind:this={plotEl}
        role="img"
        aria-label={summary}
      >
        {#if tooltip}
          <div
            class="tooltip"
            bind:offsetWidth={tipWidth}
            style:left="{tooltip.x}px"
            style:top="{tooltip.y}px"
          >
            <p class="time">{tooltip.time}</p>
            {#each tooltip.rows as row (row.key)}
              <p class="row">
                <span class="key" style:--key="var(--chart-series-{row.slot + 1})"></span>
                <strong>{row.value}</strong>
                <span class="label">{row.label}</span>
                {#if row.range}<span class="range">{row.range}</span>{/if}
              </p>
            {/each}
          </div>
        {/if}
      </div>
    {:else if data}
      <p class="state">No data in this range</p>
    {:else if error}
      <p class="state error" role="alert">Could not load: {error}</p>
    {:else}
      <div class="loading" role="status" aria-label="Loading {title}">
        <SkeletonBlock height="100%" radius="var(--radius-md)" />
      </div>
    {/if}
  </div>

  {#if error && data}<p class="note" role="alert">Update failed: {error}</p>{/if}

  {#if showTable && data && drawable}
    <div class="table-wrap" id="{id}-table">
      <table>
        <caption class="sr-only">{title}, {rangeDescription}</caption>
        <thead>
          <tr>
            <th scope="col">Time</th>
            {#each series as s (s.key)}<th scope="col"
                >{s.label} <span class="range">avg (min – max)</span></th
              >{/each}
          </tr>
        </thead>
        <tbody>
          {#each data.ts as ts, row (ts)}
            <tr>
              <th scope="row">{formatChartTime(ts, reference)}</th>
              {#each series as s, i (s.key)}
                {@const spread = spreadText(i, row)}
                <td>
                  {formatValue(unit, data.series[i]?.avg[row] ?? null)}
                  {#if spread}<span class="range">({spread})</span>{/if}
                </td>
              {/each}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</figure>

<style>
  .chart {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
    margin: 0;
    padding: var(--space-4);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-sm);
  }

  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1) var(--space-3);
  }

  .title {
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }

  .legend {
    margin-left: auto;
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1) var(--space-3);
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--text-xs);
    color: var(--color-text-muted);
  }

  .legend li {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }

  /* Line keys mirror the 2px line mark; text stays in text ink. */
  .key {
    display: inline-block;
    width: 12px;
    height: 2px;
    border-radius: 1px;
    background: var(--key);
    flex: none;
  }

  /* Clipping is a backstop: the tooltip is already positioned to stay inside. */
  .frame {
    position: relative;
    min-width: 0;
    overflow: clip;
  }

  .plot {
    position: relative;
    height: 100%;
    transition: opacity var(--duration-fast);
  }

  .refreshing {
    opacity: 0.6;
  }

  .plot :global(.u-cursor-x) {
    border-right: 1px solid var(--chart-crosshair);
  }

  .state {
    display: grid;
    place-items: center;
    height: 100%;
    font-size: var(--text-sm);
    /* text-faint is below 4.5:1 on the sunken fill (tokens.css). */
    color: var(--color-text-muted);
    background: var(--color-surface-sunken);
    border-radius: var(--radius-md);
  }

  .loading {
    height: 100%;
  }

  .state.error {
    color: var(--health-critical-fg);
  }

  .tooltip {
    position: absolute;
    z-index: 1;
    display: grid;
    gap: 2px;
    min-width: 9rem;
    padding: var(--space-2) var(--space-3);
    max-width: 100%;
    pointer-events: none;
    background: var(--color-surface-raised);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-md);
    font-size: var(--text-xs);
    white-space: nowrap;
  }

  .time {
    color: var(--color-text-faint);
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-variant-numeric: tabular-nums;
  }

  .row strong {
    font-weight: var(--weight-semibold);
    color: var(--color-text);
  }

  .label,
  .range {
    color: var(--color-text-muted);
  }

  .range {
    color: var(--color-text-faint);
  }

  .note {
    font-size: var(--text-xs);
    color: var(--health-warning-fg);
  }

  .toggle {
    margin-left: auto;
    flex: none;
    padding: 0 var(--space-2);
    font-size: var(--text-xs);
    color: var(--color-text-muted);
    background: none;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }

  .legend + .toggle {
    margin-left: 0;
  }

  .toggle:hover {
    color: var(--color-text);
    border-color: var(--color-border-strong);
  }

  .table-wrap {
    max-height: 16rem;
    overflow: auto;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
  }

  th,
  td {
    padding: var(--space-1) var(--space-2);
    text-align: right;
    white-space: nowrap;
    border-bottom: 1px solid var(--color-border);
  }

  th:first-child {
    text-align: left;
  }

  thead th {
    position: sticky;
    top: 0;
    background: var(--color-surface-raised);
    color: var(--color-text-muted);
    font-weight: var(--weight-medium);
  }

  tbody th {
    font-weight: var(--weight-regular);
    color: var(--color-text-muted);
  }
</style>
