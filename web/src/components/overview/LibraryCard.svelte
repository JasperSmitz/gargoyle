<script lang="ts">
  import type { LibrarySummary } from '../../lib/api/types';
  import { formatRelativeTime } from '../../lib/format';
  import { needsAttention } from '../../lib/health';
  import { SEVERITY_ORDER, severityCount } from '../../lib/library';
  import { href } from '../../lib/router';
  import SeverityChip from '../library/SeverityChip.svelte';
  import Section from '../Section.svelte';
  import StatCard from '../StatCard.svelte';
  import Time from '../Time.svelte';

  // The music library on the overview; only rendered when `[library]` is configured.
  let { library, serverNow }: { library: LibrarySummary; serverNow: number } = $props();
</script>

<Section title="Library">
  <StatCard title={library.name} health={library.health}>
    {#if library.message && needsAttention(library.health)}
      <p class="message">{library.message}</p>
    {/if}
    {#if library.issueCounts}
      <ul class="counts" aria-label="Issues by severity">
        {#each SEVERITY_ORDER as severity (severity)}
          <li>
            <SeverityChip
              {severity}
              label={severityCount(severity, library.issueCounts[severity])}
            />
          </li>
        {/each}
      </ul>
    {/if}
    <p class="meta">
      {#if library.unreadableFiles !== null}
        <span
          >{library.unreadableFiles}
          {library.unreadableFiles === 1 ? 'unreadable file' : 'unreadable files'}</span
        >
      {/if}
      {#if library.albums !== null}
        <span>{library.albums} {library.albums === 1 ? 'album' : 'albums'}</span>
      {/if}
      {#if library.generatedAt !== null}
        <span
          >Report <Time
            ts={library.generatedAt}
            text={formatRelativeTime(library.generatedAt, serverNow)}
          /></span
        >
      {/if}
    </p>
    <a class="more" href={href('library')}>View library report</a>
  </StatCard>
</Section>

<style>
  .message {
    font-size: var(--text-sm);
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }

  .counts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1) var(--space-3);
    font-size: var(--text-sm);
    color: var(--color-text-muted);
  }

  .more {
    align-self: flex-start;
    font-size: var(--text-sm);
  }
</style>
