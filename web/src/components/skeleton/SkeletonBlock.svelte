<script lang="ts">
  // One placeholder shape. Page skeletons compose these at the size of what will replace them,
  // so nothing jumps when data arrives. Purely decorative: the wrapping status names the wait.
  let {
    height,
    width = '100%',
    radius = 'var(--radius-lg)',
  }: { height: string; width?: string; radius?: string } = $props();
</script>

<div class="block" aria-hidden="true" style:height style:width style:border-radius={radius}></div>

<style>
  .block {
    max-width: 100%;
    background: var(--color-surface-sunken);
    animation: pulse 1.4s ease-in-out infinite;
  }

  @keyframes pulse {
    50% {
      opacity: 0.55;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .block {
      animation: none;
    }
  }
</style>
