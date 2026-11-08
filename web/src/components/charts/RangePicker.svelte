<script lang="ts">
  import { RANGES, type RangeId } from '../../lib/charts/range';

  // Links rather than buttons: the range lives in the URL, so each option is a place to go.
  let { selected, hrefFor }: { selected: RangeId; hrefFor: (range: RangeId) => string } = $props();
</script>

<nav class="picker" aria-label="Time range">
  {#each RANGES as range (range.id)}
    <a
      class="option"
      href={hrefFor(range.id)}
      aria-current={range.id === selected ? 'true' : undefined}
      title={range.description}>{range.id}</a
    >
  {/each}
</nav>

<style>
  .picker {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 2px;
    padding: 2px;
    background: var(--color-surface-sunken);
    border-radius: var(--radius-md);
  }

  .option {
    min-width: 2.75rem;
    padding: var(--space-1) var(--space-3);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    text-align: center;
    color: var(--color-text-muted);
    text-decoration: none;
    border-radius: var(--radius-sm);
  }

  .option:hover {
    color: var(--color-text);
    background: color-mix(in srgb, var(--color-surface) 50%, transparent);
  }

  .option[aria-current='true'] {
    font-weight: var(--weight-semibold);
    color: var(--color-text);
    background: var(--color-surface-raised);
    box-shadow: var(--shadow-sm);
  }
</style>
