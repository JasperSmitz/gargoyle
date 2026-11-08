<script lang="ts">
  import SkeletonBlock from './SkeletonBlock.svelte';

  // First-load placeholder with the overview's shape, so content does not jump when it arrives.
</script>

<div class="skeleton" role="status" aria-label="Loading overview">
  <SkeletonBlock height="6.5rem" />
  <div class="grid">
    {#each { length: 6 }, i (i)}
      <SkeletonBlock height="8.5rem" />
    {/each}
  </div>
  <SkeletonBlock height="14rem" />
</div>

<style>
  .skeleton {
    display: grid;
    gap: var(--space-6);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 14rem), 1fr));
    gap: var(--space-3);
  }
</style>
