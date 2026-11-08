<script lang="ts">
  // The overview before any data has ever arrived, when the first request failed. The banner
  // above says why and offers a retry; this keeps the page from being a lone banner and gives it
  // a heading to land on.
  let { unauthorized = false }: { unauthorized?: boolean } = $props();
</script>

<section class="shell" aria-labelledby="offline-heading">
  <h1 id="offline-heading">{unauthorized ? 'Sign-in needed' : 'Waiting for the server'}</h1>
  <p>
    {#if unauthorized}
      The dashboard appears once you have signed in.
    {:else}
      The dashboard appears as soon as the Gargoyle server answers. Nothing is shown until then,
      because there is no earlier data in this browser to fall back on.
    {/if}
  </p>
</section>

<style>
  .shell {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-6) var(--space-4);
    border: 1px dashed var(--color-border-strong);
    border-radius: var(--radius-lg);
    text-align: center;
  }

  h1 {
    font-size: var(--text-xl);
  }

  p {
    max-width: 36rem;
    margin-inline: auto;
    color: var(--color-text-muted);
  }
</style>
