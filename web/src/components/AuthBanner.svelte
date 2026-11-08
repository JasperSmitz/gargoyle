<script lang="ts">
  import HealthIcon from './HealthIcon.svelte';

  // Replaces the stale banner when the server answers 401: retrying cannot help, only a reload
  // makes the browser ask for credentials again.
  let { onreload = () => location.reload() }: { onreload?: () => void } = $props();
</script>

<div class="banner">
  <HealthIcon health="critical" size={18} />
  <div class="text" role="alert">
    <p class="title">Authentication required — reload to sign in</p>
    <p class="detail">The server no longer accepts this browser's credentials.</p>
  </div>
  <button class="reload" type="button" onclick={onreload}>Reload</button>
</div>

<style>
  .banner {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    gap: var(--space-2) var(--space-3);
    padding: var(--space-3) var(--space-4);
    color: var(--health-critical-fg);
    background: var(--health-critical-bg);
    border: 1px solid var(--health-critical-border);
    border-radius: var(--radius-lg);
  }

  .banner > :global(.icon) {
    margin-top: 0.15rem;
  }

  .text {
    flex: 1 1 12rem;
    min-width: 0;
  }

  .title {
    font-weight: var(--weight-semibold);
  }

  .detail {
    font-size: var(--text-sm);
    color: var(--color-text);
  }

  .reload {
    padding: var(--space-1) var(--space-3);
    border: 1px solid var(--health-critical-border);
    border-radius: var(--radius-md);
    background: var(--color-surface);
    color: var(--color-text);
    cursor: pointer;
  }

  .reload:hover {
    background: var(--color-surface-raised);
  }
</style>
