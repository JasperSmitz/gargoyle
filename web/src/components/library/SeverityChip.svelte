<script lang="ts">
  import type { Severity } from '../../lib/api/types';
  import { SEVERITY_PRESENTATION } from '../../lib/library';

  // A Liner severity as icon + text + color, so it reads without color vision. `label` replaces
  // the severity name (e.g. "2 errors"); the icon still conveys which severity it is.
  let { severity, label }: { severity: Severity; label?: string } = $props();

  const icon = $derived(SEVERITY_PRESENTATION[severity].icon);
</script>

<span class="chip severity-{severity}" data-severity={severity}>
  <svg
    width="12"
    height="12"
    viewBox="0 0 16 16"
    aria-hidden="true"
    focusable="false"
    data-icon={icon}
  >
    {#if icon === 'octagon'}
      <path d="M5.25 1.25h5.5l4 4v5.5l-4 4h-5.5l-4-4v-5.5z" />
      <path d="M5.75 5.75l4.5 4.5M10.25 5.75l-4.5 4.5" />
    {:else if icon === 'alert'}
      <path d="M8 1.75l6.5 12H1.5z" />
      <path d="M8 6.25v3.5M8 11.75v.01" />
    {:else}
      <circle cx="8" cy="8" r="6.75" />
      <path d="M8 7.25v4M8 4.75v.01" />
    {/if}
  </svg>
  <span>{label ?? SEVERITY_PRESENTATION[severity].label}</span>
</span>

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    flex: none;
    padding: 0.125rem var(--space-2) 0.125rem 0.375rem;
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
    line-height: 1;
    white-space: nowrap;
    border: 1px solid;
    border-radius: var(--radius-pill);
  }

  svg {
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .severity-error {
    color: var(--severity-error-fg);
    background: var(--severity-error-bg);
    border-color: var(--severity-error-border);
  }

  .severity-warning {
    color: var(--severity-warning-fg);
    background: var(--severity-warning-bg);
    border-color: var(--severity-warning-border);
  }

  .severity-info {
    color: var(--severity-info-fg);
    background: var(--severity-info-bg);
    border-color: var(--severity-info-border);
  }
</style>
