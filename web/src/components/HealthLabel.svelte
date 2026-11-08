<script lang="ts">
  import type { Health } from '../lib/api/types';
  import { HEALTH_PRESENTATION } from '../lib/health';
  import HealthIcon from './HealthIcon.svelte';

  // Inline counterpart of HealthBadge for dense spots (event transitions, sensor lists, problem
  // lines): the state's icon and color with its name as text. `hideText` keeps the name for
  // screen readers only, where the surrounding layout already makes the state obvious visually.
  let {
    health,
    hideText = false,
    size = 12,
  }: { health: Health; hideText?: boolean; size?: number } = $props();

  const label = $derived(HEALTH_PRESENTATION[health].label);
</script>

<span class="label health-{health}" data-health={health}>
  <HealthIcon {health} {size} />
  <span class:sr-only={hideText}>{label}</span>
</span>

<style>
  .label {
    display: inline-flex;
    align-items: center;
    gap: 0.2rem;
    font-weight: var(--weight-medium);
    white-space: nowrap;
  }

  .health-healthy {
    color: var(--health-healthy-fg);
  }
  .health-warning {
    color: var(--health-warning-fg);
  }
  .health-critical {
    color: var(--health-critical-fg);
  }
  .health-unknown {
    color: var(--health-unknown-fg);
  }
</style>
