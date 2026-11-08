<script lang="ts">
  // First-load placeholder with the overview's shape, so content does not jump when it arrives.
</script>

<div class="skeleton" role="status" aria-label="Loading overview">
  <div class="block summary"></div>
  <div class="grid">
    {#each { length: 6 }, i (i)}
      <div class="block card"></div>
    {/each}
  </div>
  <div class="block list"></div>
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

  .block {
    border-radius: var(--radius-lg);
    background: var(--color-surface-sunken);
    animation: pulse 1.4s ease-in-out infinite;
  }

  .summary {
    height: 6.5rem;
  }

  .card {
    height: 8.5rem;
  }

  .list {
    height: 14rem;
  }

  @keyframes pulse {
    50% {
      opacity: 0.55;
    }
  }
</style>
