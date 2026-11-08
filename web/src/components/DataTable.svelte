<script lang="ts">
  import type { Snippet } from 'svelte';

  // The overview's target tables (timers, checks): a table on wide screens, one card per row on
  // narrow ones. Rows are written by the caller with these conventions:
  //   <tr data-health>  <th scope="row" class="name-cell">  <td data-label="…">  <td class="health-cell">
  // and inside the name cell `.name` (link), `.id` (technical id), `.message` (why not healthy).
  // Cells may use `.sub` (secondary line) and `.never` (an absent value, italic).
  let { head, children }: { head: Snippet; children: Snippet } = $props();
</script>

<table class="table">
  <thead><tr>{@render head()}</tr></thead>
  <tbody>{@render children()}</tbody>
</table>

<style>
  .table {
    width: 100%;
    border-collapse: separate;
    border-spacing: 0;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-sm);
    font-size: var(--text-sm);
    overflow: hidden;
  }

  .table :global(th),
  .table :global(td) {
    padding: var(--space-3) var(--space-4);
    text-align: left;
    vertical-align: top;
    border-bottom: 1px solid var(--color-border);
  }

  .table :global(tbody tr:last-child > *) {
    border-bottom: none;
  }

  thead :global(th) {
    padding-block: var(--space-2);
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
    text-transform: uppercase;
    letter-spacing: var(--tracking-wide);
    background: var(--color-surface-sunken);
    color: var(--color-text-muted);
  }

  .table :global(td) {
    font-variant-numeric: tabular-nums;
    color: var(--color-text-muted);
  }

  .table :global(td[data-label] > span) {
    display: block;
  }

  .table :global(.name-cell) {
    font-weight: var(--weight-regular);
    min-width: 0;
  }

  .table :global([data-health='warning'] > .name-cell) {
    box-shadow: inset 3px 0 0 var(--health-warning-fg);
  }
  .table :global([data-health='critical'] > .name-cell) {
    box-shadow: inset 3px 0 0 var(--health-critical-fg);
  }
  .table :global([data-health='unknown'] > .name-cell) {
    box-shadow: inset 3px 0 0 var(--health-unknown-fg);
  }

  .table :global(.name) {
    display: block;
    font-size: var(--text-md);
    font-weight: var(--weight-medium);
    color: inherit;
    text-decoration-color: var(--color-border-strong);
    text-underline-offset: 0.2em;
    overflow-wrap: anywhere;
  }

  .table :global(.name:hover) {
    color: var(--color-accent-hover);
    text-decoration-color: currentColor;
  }

  .table :global(.id) {
    display: block;
    font-size: var(--text-xs);
    color: var(--color-text-faint);
    overflow-wrap: anywhere;
  }

  .table :global(.message) {
    margin-top: var(--space-1);
    overflow-wrap: anywhere;
  }

  .table :global([data-health='warning'] .message) {
    color: var(--health-warning-fg);
  }
  .table :global([data-health='critical'] .message) {
    color: var(--health-critical-fg);
  }
  .table :global([data-health='unknown'] .message) {
    color: var(--health-unknown-fg);
  }

  .table :global(td[data-label] > span:first-child) {
    color: var(--color-text);
  }

  .table :global(.sub) {
    font-size: var(--text-xs);
  }

  .table :global(.never) {
    font-style: italic;
  }

  .table :global(.health-cell) {
    text-align: right;
  }

  /* Narrow screens: one card per row, name and health on top, labelled cells below. */
  @media (max-width: 40rem) {
    /* Without this the grid rows shrink to fit inside the table box. */
    .table,
    tbody {
      display: block;
    }

    thead {
      position: absolute;
      width: 1px;
      height: 1px;
      overflow: hidden;
      clip-path: inset(50%);
    }

    /* Name and badge on the first line, the labelled cells side by side below. */
    tbody :global(tr) {
      display: flex;
      flex-wrap: wrap;
      align-items: flex-start;
      border-bottom: 1px solid var(--color-border);
    }

    tbody :global(tr:last-child) {
      border-bottom: none;
    }

    .table :global(th),
    .table :global(td) {
      border-bottom: none;
    }

    /* Wide enough that a labelled cell can never join the first line. */
    .table :global(.name-cell) {
      flex: 1 1 60%;
      order: 0;
      padding-bottom: var(--space-1);
    }

    .table :global(.health-cell) {
      flex: none;
      order: 1;
      padding-bottom: var(--space-1);
    }

    .table :global(td[data-label]) {
      flex: 1 1 30%;
      order: 2;
      padding-top: 0;
    }

    .table :global(td[data-label]::before) {
      content: attr(data-label);
      display: block;
      font-size: var(--text-xs);
      color: var(--color-text-faint);
    }

    .table :global([data-health] > .name-cell) {
      box-shadow: none;
    }

    tbody :global(tr[data-health='warning']) {
      box-shadow: inset 3px 0 0 var(--health-warning-fg);
    }
    tbody :global(tr[data-health='critical']) {
      box-shadow: inset 3px 0 0 var(--health-critical-fg);
    }
    tbody :global(tr[data-health='unknown']) {
      box-shadow: inset 3px 0 0 var(--health-unknown-fg);
    }
  }
</style>
