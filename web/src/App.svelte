<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './lib/api/client';
  import type { HealthResponse } from './lib/api/types';

  // Placeholder shell (M0.5). M1.6 adds hash routing, the overview page and polling.
  type Status =
    | { kind: 'loading' }
    | { kind: 'ok'; health: HealthResponse }
    | { kind: 'error'; message: string };

  let status = $state<Status>({ kind: 'loading' });

  async function load() {
    status = { kind: 'loading' };
    try {
      status = { kind: 'ok', health: await api.health() };
    } catch (error) {
      status = { kind: 'error', message: error instanceof Error ? error.message : String(error) };
    }
  }

  onMount(load);
</script>

<header class="header">
  <div class="header-inner">
    <span class="wordmark">Gargoyle</span>
  </div>
</header>

<main class="main">
  <section class="card" aria-live="polite" aria-busy={status.kind === 'loading'}>
    {#if status.kind === 'loading'}
      <p class="muted">Contacting the server…</p>
    {:else if status.kind === 'ok'}
      <h1 class="title">Watching</h1>
      <p class="muted">
        Backend reachable · version <code>{status.health.version}</code>
      </p>
    {:else}
      <h1 class="title error">Server unreachable</h1>
      <p class="muted">{status.message}</p>
      <button class="button" type="button" onclick={load}>Retry</button>
    {/if}
  </section>
</main>

<style>
  .header {
    height: var(--header-height);
    border-bottom: 1px solid var(--color-border);
    background: var(--color-surface);
  }

  .header-inner,
  .main {
    max-width: var(--content-max-width);
    margin: 0 auto;
    padding-inline: var(--space-4);
  }

  .header-inner {
    display: flex;
    align-items: center;
    height: 100%;
  }

  .wordmark {
    font-size: var(--text-lg);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-wide);
  }

  .main {
    padding-block: var(--space-6);
  }

  .card {
    display: grid;
    gap: var(--space-2);
    justify-items: start;
    padding: var(--space-5);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-sm);
  }

  .title {
    font-size: var(--text-xl);
  }

  .title.error {
    color: var(--health-critical-fg);
  }

  .muted {
    color: var(--color-text-muted);
  }

  .button {
    margin-top: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border: 1px solid var(--color-accent);
    border-radius: var(--radius-md);
    background: var(--color-accent);
    color: var(--color-on-accent);
    cursor: pointer;
    transition: background-color var(--duration-fast);
  }

  .button:hover {
    background: var(--color-accent-hover);
  }
</style>
