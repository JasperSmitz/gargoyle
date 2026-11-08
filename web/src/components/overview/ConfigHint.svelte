<script lang="ts">
  import GargoyleMark from '../GargoyleMark.svelte';

  type Kind = 'service' | 'timer' | 'check';

  // What the overview shows for target kinds with nothing configured. With none configured at
  // all it is a calm panel, titled "all quiet" when healthy, saying system metrics are still
  // watched; otherwise one quiet line naming the kinds left out, so a host without timers does
  // not get an empty box for them.
  let {
    missing,
    total,
    healthy,
  }: {
    missing: Kind[];
    total: number;
    /** "All quiet" would contradict a system warning, so the heading depends on it. */
    healthy: boolean;
  } = $props();

  const NOUN: Record<Kind, string> = { service: 'services', timer: 'timers', check: 'checks' };
  const titleId = $props.id();
  const all = $derived(missing.length === total);

  /** Separator before item `i` of `n`: "a", "a or b", "a, b or c". */
  const sep = (i: number, n: number) => (i === 0 ? '' : i === n - 1 ? ' or ' : ', ');
</script>

{#snippet sections()}
  {#each missing as kind, i (kind)}{sep(i, missing.length)}<code>[[{kind}]]</code>{/each}
{/snippet}

{#if all}
  <section class="quiet" aria-labelledby={titleId}>
    <GargoyleMark size={40} />
    <div class="text">
      <h2 class="title" id={titleId}>
        {healthy ? 'All quiet' : 'Only system metrics are watched'}
      </h2>
      <p>
        Gargoyle is watching this host's system metrics. To also watch services, timers or HTTP/TCP
        checks, add {@render sections()} entries to <code>gargoyle.toml</code> and restart Gargoyle;
        <code>config/gargoyle.example.toml</code> documents every option.
      </p>
    </div>
  </section>
{:else if missing.length > 0}
  <p class="line">
    Not watching any
    {#each missing as kind, i (kind)}{sep(i, missing.length)}{NOUN[kind]}{/each}. Add
    {@render sections()} entries to <code>gargoyle.toml</code> to include them.
  </p>
{/if}

<style>
  .quiet {
    display: flex;
    align-items: flex-start;
    gap: var(--space-4);
    padding: var(--space-5);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
  }

  .text {
    display: grid;
    gap: var(--space-1);
    max-width: 40rem;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .title {
    font-size: var(--text-md);
    color: var(--color-text);
  }

  .line {
    font-size: var(--text-sm);
    color: var(--color-text-faint);
  }
</style>
