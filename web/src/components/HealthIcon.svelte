<script lang="ts">
  import type { Health } from '../lib/api/types';
  import { HEALTH_PRESENTATION } from '../lib/health';

  // Decorative: always rendered next to the state's text label, so hidden from screen readers.
  let { health, size = 16 }: { health: Health; size?: number } = $props();

  const icon = $derived(HEALTH_PRESENTATION[health].icon);
</script>

<svg
  class="icon"
  width={size}
  height={size}
  viewBox="0 0 16 16"
  aria-hidden="true"
  focusable="false"
  data-icon={icon}
>
  {#if icon === 'check'}
    <circle cx="8" cy="8" r="6.75" />
    <path d="M5 8.25l2 2 4-4.25" />
  {:else if icon === 'alert'}
    <path d="M8 1.75l6.5 12H1.5z" />
    <path d="M8 6.25v3.5M8 11.75v.01" />
  {:else if icon === 'octagon'}
    <path d="M5.25 1.25h5.5l4 4v5.5l-4 4h-5.5l-4-4v-5.5z" />
    <path d="M5.75 5.75l4.5 4.5M10.25 5.75l-4.5 4.5" />
  {:else}
    <circle cx="8" cy="8" r="6.75" />
    <path d="M6.1 6.2a2 2 0 1 1 2.6 1.9c-.5.2-.7.6-.7 1.1v.3M8 11.75v.01" />
  {/if}
</svg>

<style>
  .icon {
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>
