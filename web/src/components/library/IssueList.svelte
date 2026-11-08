<script lang="ts">
  import type { LibraryIssue } from '../../lib/api/types';
  import { displayPath, morePaths } from '../../lib/library';
  import SeverityChip from './SeverityChip.svelte';

  // Issues of one album (or of the whole library). Messages and paths come from the report and
  // are rendered as text only: plain interpolations, which Svelte escapes (csp.test.ts checks).
  // Paths are one text node each, wrapped by `overflow-wrap: anywhere`, without <wbr> break
  // points: a report can hold 25,000 paths, and per-separator elements multiplied into millions.
  let {
    issues,
    albumId = null,
  }: {
    issues: LibraryIssue[];
    /** Paths inside this album directory are shown relative to it. */
    albumId?: string | null;
  } = $props();
</script>

<ul class="issues">
  {#each issues as issue, i (i)}
    {@const more = morePaths(issue)}
    <li class="issue">
      <div class="head">
        <SeverityChip severity={issue.severity} />
        <code class="code">{issue.code}</code>
      </div>
      <p class="message">{issue.message}</p>
      {#if issue.paths.length > 0}
        <ul class="paths" aria-label="Files">
          {#each issue.paths as path, j (j)}
            {@const shown = displayPath(path, albumId)}
            <li title={shown === path ? undefined : path}>
              {shown}
            </li>
          {/each}
          {#if more}<li class="more">{more}</li>{/if}
        </ul>
      {/if}
    </li>
  {/each}
</ul>

<style>
  .issues {
    display: grid;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .issue {
    display: grid;
    gap: var(--space-1);
    min-width: 0;
    padding-block: var(--space-3);
    border-top: 1px solid var(--color-border);
  }

  .issue:first-child {
    border-top: none;
  }

  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1) var(--space-2);
  }

  .code {
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }

  .message {
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }

  .paths {
    display: grid;
    gap: 2px;
    margin: 0;
    padding: 0 0 0 var(--space-3);
    list-style: none;
    border-left: 2px solid var(--color-border);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--color-text-muted);
  }

  .paths li {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .more {
    font-family: var(--font-sans);
    font-style: italic;
  }
</style>
