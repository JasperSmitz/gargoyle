<script lang="ts">
  import SkeletonBlock from './SkeletonBlock.svelte';

  // Rows of a CardList (events, runs) while they load.
  let { rows = 6, label }: { rows?: number; label: string } = $props();
</script>

<div class="list" role="status" aria-label={label}>
  {#each { length: rows }, i (i)}
    <div class="row">
      <SkeletonBlock height="1rem" width="3.5rem" radius="var(--radius-sm)" />
      <SkeletonBlock height="1rem" width="{55 + ((i * 17) % 35)}%" radius="var(--radius-sm)" />
    </div>
  {/each}
</div>

<style>
  .list {
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    overflow: hidden;
  }

  .row {
    display: grid;
    grid-template-columns: 4.5rem 1fr;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--color-border);
  }

  .row:last-child {
    border-bottom: none;
  }
</style>
