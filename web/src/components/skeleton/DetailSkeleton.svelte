<script lang="ts">
  import SkeletonBlock from './SkeletonBlock.svelte';
  import ChartsSkeleton from './ChartsSkeleton.svelte';
  import ListSkeleton from './ListSkeleton.svelte';

  // A detail page's sections before its first response (DetailHeader shows its own placeholder).
  // Each section keeps its heading row (title, range picker) so nothing shifts on arrival.
  let {
    noun,
    charts = 0,
    runs = false,
  }: {
    noun: string;
    /** Chart panels the page will show; 0 for none. */
    charts?: number;
    /** A run history list (timers). */
    runs?: boolean;
  } = $props();
</script>

{#if runs}
  <div class="section">
    <SkeletonBlock height="1.25rem" width="8rem" radius="var(--radius-sm)" />
    <ListSkeleton rows={6} label="Loading run history" />
  </div>
{/if}
{#if charts > 0}
  <div class="section">
    <SkeletonBlock height="2rem" width="8rem" radius="var(--radius-sm)" />
    <ChartsSkeleton count={charts} label="Loading {noun} history" />
  </div>
{/if}
<div class="section">
  <SkeletonBlock height="1.25rem" width="6rem" radius="var(--radius-sm)" />
  <ListSkeleton rows={3} label="Loading {noun} events" />
</div>

<style>
  .section {
    display: grid;
    gap: var(--space-3);
  }
</style>
