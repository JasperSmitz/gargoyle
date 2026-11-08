<script lang="ts">
  import type { Health } from '../lib/api/types';
  import { HEALTH_PRESENTATION } from '../lib/health';
  import HealthIcon from './HealthIcon.svelte';

  // Color + icon shape + text, so the state reads without color vision (ROADMAP M4.4).
  let {
    health,
    size = 'md',
    label,
  }: {
    health: Health;
    size?: 'sm' | 'md' | 'lg';
    /** Replaces the state name, e.g. "2 critical"; the icon still conveys the state. */
    label?: string;
  } = $props();
</script>

<span class="badge {size} health-{health}" data-health={health}>
  <HealthIcon {health} size={size === 'lg' ? 18 : size === 'sm' ? 12 : 14} />
  <span>{label ?? HEALTH_PRESENTATION[health].label}</span>
</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    flex: none;
    border: 1px solid;
    border-radius: var(--radius-pill);
    font-weight: var(--weight-medium);
    line-height: 1;
    white-space: nowrap;
  }

  .sm {
    padding: 0.125rem var(--space-2) 0.125rem 0.375rem;
    font-size: var(--text-xs);
  }

  .md {
    padding: var(--space-1) var(--space-2) var(--space-1) 0.375rem;
    font-size: var(--text-sm);
  }

  .lg {
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3) var(--space-2) var(--space-2);
    font-size: var(--text-md);
    font-weight: var(--weight-semibold);
  }

  .health-healthy {
    color: var(--health-healthy-fg);
    background: var(--health-healthy-bg);
    border-color: var(--health-healthy-border);
  }

  .health-warning {
    color: var(--health-warning-fg);
    background: var(--health-warning-bg);
    border-color: var(--health-warning-border);
  }

  .health-critical {
    color: var(--health-critical-fg);
    background: var(--health-critical-bg);
    border-color: var(--health-critical-border);
  }

  .health-unknown {
    color: var(--health-unknown-fg);
    background: var(--health-unknown-bg);
    border-color: var(--health-unknown-border);
  }
</style>
