<script lang="ts">
  import type { Health } from '../lib/api/types';

  let {
    percent,
    health = 'healthy',
    label,
  }: {
    percent: number;
    health?: Health;
    /** Accessible name, e.g. "Disk / usage". */
    label: string;
  } = $props();

  const clamped = $derived(Math.min(100, Math.max(0, Number.isFinite(percent) ? percent : 0)));
</script>

<div
  class="track"
  role="meter"
  aria-label={label}
  aria-valuemin="0"
  aria-valuemax="100"
  aria-valuenow={Math.round(clamped)}
>
  <div class="fill health-{health}" style:width="{clamped}%"></div>
</div>

<style>
  .track {
    height: 0.375rem;
    overflow: hidden;
    border-radius: var(--radius-pill);
    background: var(--color-surface-sunken);
  }

  .fill {
    height: 100%;
    border-radius: inherit;
    transition: width var(--duration-fast) ease-out;
  }

  /* Healthy usage stays calm (accent); only problems take on a health color. */
  .health-healthy {
    background: var(--color-accent);
  }

  .health-warning {
    background: var(--health-warning-fg);
  }

  .health-critical {
    background: var(--health-critical-fg);
  }

  .health-unknown {
    background: var(--health-unknown-fg);
  }
</style>
