<script lang="ts">
  import type { SeriesUnit } from '../../lib/api/types';
  import { type Column, extent } from '../../lib/charts/data';
  import { sparklinePath } from '../../lib/charts/sparkline';
  import { formatValue } from '../../lib/charts/units';

  // A trend glance for stat cards: no axes, no tooltip; the card's number is the value.
  let {
    values,
    unit,
    label,
  }: {
    values: Column;
    unit: SeriesUnit;
    /** What is plotted, e.g. "CPU usage"; the accessible name adds the period and extent. */
    label: string;
  } = $props();

  const WIDTH = 120;
  const HEIGHT = 28;
  const MIN_SPAN: Record<SeriesUnit, number> = {
    percent: 10,
    load: 0.5,
    bytes: 1024 * 1024,
    bytes_per_s: 1024,
    celsius: 5,
    ms: 5,
    count: 1,
  };

  const geometry = $derived(sparklinePath(values, WIDTH, HEIGHT, MIN_SPAN[unit]));
  const span = $derived(extent([values]));
  const description = $derived(
    span
      ? `${label}, last hour: ${formatValue(unit, span.min)} to ${formatValue(unit, span.max)}`
      : `${label}, last hour: no data`,
  );
</script>

{#if geometry.end}
  <svg
    class="spark"
    viewBox="-3 -3 {WIDTH + 6} {HEIGHT + 6}"
    preserveAspectRatio="none"
    role="img"
    aria-label={description}
  >
    <path d={geometry.path} />
    <circle cx={geometry.end.x} cy={geometry.end.y} r="2.5" />
  </svg>
{/if}

<style>
  .spark {
    display: block;
    width: 100%;
    height: 1.75rem;
    overflow: visible;
  }

  path {
    fill: none;
    stroke: var(--chart-spark);
    stroke-width: 1.5;
    stroke-linejoin: round;
    stroke-linecap: round;
    vector-effect: non-scaling-stroke;
  }

  /* The current value in the accent; the history recedes. */
  circle {
    fill: var(--color-accent);
  }
</style>
