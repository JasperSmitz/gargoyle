<script lang="ts">
  import SkeletonBlock from './SkeletonBlock.svelte';

  // Chart panels while the series list loads; same grid and height as TimeSeriesChart.
  let { count = 4, label = 'Loading charts' }: { count?: number; label?: string } = $props();
</script>

<div class="charts" role="status" aria-label={label}>
  {#each { length: count }, i (i)}
    <div class="panel">
      <SkeletonBlock height="1.25rem" width="8rem" radius="var(--radius-sm)" />
      <SkeletonBlock height="200px" radius="var(--radius-md)" />
    </div>
  {/each}
</div>

<style>
  .charts {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 28rem), 1fr));
    gap: var(--space-3);
  }

  .panel {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-4);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
  }
</style>
