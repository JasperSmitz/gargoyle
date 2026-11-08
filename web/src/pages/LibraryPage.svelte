<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import DetailHeader from '../components/detail/DetailHeader.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import IssueList from '../components/library/IssueList.svelte';
  import SeverityChip from '../components/library/SeverityChip.svelte';
  import Section from '../components/Section.svelte';
  import ListSkeleton from '../components/skeleton/ListSkeleton.svelte';
  import Time from '../components/Time.svelte';
  import type { ApiClient } from '../lib/api/client';
  import type { LibraryReport } from '../lib/api/types';
  import { PolledFeed } from '../lib/feed.svelte';
  import { formatRelativeTime } from '../lib/format';
  import {
    SEVERITY_FILTERS,
    SEVERITY_ORDER,
    countBySeverity,
    filterAlbums,
    filterIssues,
    severityCount,
    type SeverityFilter,
  } from '../lib/library';
  import { describeFeedError } from '../lib/overview-feed.svelte';
  import type { Visibility } from '../lib/poller';
  import { hrefWithQuery } from '../lib/router';

  // The music library report (ADR 0007). The report changes about once a day, so 60s polling is
  // plenty. The h1 stays mounted through every state so route focus is never lost.
  let {
    filter,
    client,
    visibility,
    serverNow,
    name = 'Music library',
  }: {
    filter: SeverityFilter;
    client: Pick<ApiClient, 'library'>;
    visibility?: Visibility;
    serverNow: number;
    /** Heading until the response names the library (the overview may already know it). */
    name?: string;
  } = $props();

  const feed = new PolledFeed((signal) => untrack(() => client).library({ signal }), {
    visibility: untrack(() => visibility),
  });

  onMount(() => {
    feed.start();
    return () => feed.stop();
  });

  const response = $derived(feed.data);
  const library = $derived(response?.library ?? null);
  const report = $derived(response?.report ?? null);
  const failure = $derived(feed.error === null ? null : describeFeedError(feed.error));

  const albums = $derived(report ? filterAlbums(report.albums, filter) : []);
  const libraryIssues = $derived(report ? filterIssues(report.libraryIssues, filter) : []);
  const shownIssues = $derived(
    albums.reduce((n, a) => n + a.issues.length, 0) + libraryIssues.length,
  );

  // Open albums by id; native <details> otherwise, so keyboard and find-in-page work as usual.
  const expanded = new SvelteSet<string>();
  const expandAll = () => albums.forEach((a) => expanded.add(a.id));
  const collapseAll = () => expanded.clear();

  function keptIssues(r: LibraryReport): number {
    return r.albums.reduce((n, a) => n + a.issues.length, 0) + r.libraryIssues.length;
  }

  function tiles(r: LibraryReport) {
    const s = r.summary;
    return [
      { label: 'Albums', value: s.albums },
      { label: 'Audio files', value: s.audioFiles },
      { label: 'Unreadable files', value: s.unreadableFiles },
      { label: 'Errors', value: s.issueCounts.error, severity: 'error' as const },
      { label: 'Warnings', value: s.issueCounts.warning, severity: 'warning' as const },
      { label: 'Info', value: s.issueCounts.info, severity: 'info' as const },
    ];
  }
</script>

<div class="page">
  <DetailHeader
    kind="Music library"
    name={library?.name ?? name}
    unit={library?.name ?? name}
    health={library?.health ?? null}
    message={library?.message ?? null}
    loading={response === null && feed.error === null ? 'Loading library' : null}
  >
    {#snippet facts()}
      {#if library}
        <div>
          <dt>Report written</dt>
          <dd>
            {#if library.generatedAt !== null}
              <Time
                ts={library.generatedAt}
                text={formatRelativeTime(library.generatedAt, serverNow)}
              />
              <span class="sub"><Time ts={library.generatedAt} /></span>
            {:else}
              No report
            {/if}
          </dd>
        </div>
        <div>
          <dt>Report tool</dt>
          <dd>
            {#if report}
              {report.tool.name}
              {report.tool.version}
              <span class="sub">schema {report.schemaVersion}</span>
            {:else}
              —
            {/if}
          </dd>
        </div>
        <div>
          <dt>Last checked</dt>
          <dd>
            {#if library.checkedAt !== null}
              <Time
                ts={library.checkedAt}
                text={formatRelativeTime(library.checkedAt, serverNow)}
              />
            {:else}
              Not yet
            {/if}
          </dd>
        </div>
      {/if}
    {/snippet}
  </DetailHeader>

  {#if response !== null && failure}
    <p class="note warn" role="alert">
      Refresh failed: {failure.detail}. Showing the last data received.
    </p>
  {/if}

  {#if response === null}
    {#if failure}
      <EmptyState title="Could not load the library report">
        {failure.detail}.
        <button type="button" class="retry" onclick={() => feed.refresh()}>Try again</button>
      </EmptyState>
    {:else}
      <ListSkeleton rows={4} label="Loading library issues" />
    {/if}
  {:else if !response.configured || library === null}
    <EmptyState title="No music library configured">
      Add a <code>[library]</code> section naming a Liner report to <code>gargoyle.toml</code> and
      restart Gargoyle; <code>docs/CONFIG.md</code> documents it.
    </EmptyState>
  {:else if report === null}
    <EmptyState title="No report to show">
      {library.message ?? 'The report has not been read yet.'}
    </EmptyState>
  {:else}
    <section aria-label="Report summary">
      <dl class="tiles">
        {#each tiles(report) as tile (tile.label)}
          <div class="tile">
            <dt>{tile.label}</dt>
            <dd>
              {#if tile.severity && tile.value > 0}
                <SeverityChip severity={tile.severity} label={String(tile.value)} />
              {:else}
                {tile.value}
              {/if}
            </dd>
          </div>
        {/each}
      </dl>
    </section>

    {#if report.summary.issuesHidden > 0 || report.issuesTruncated || report.errorsTruncated}
      <ul class="note notes">
        {#if report.summary.issuesHidden > 0}
          <li>
            The report was written with <code>--min-severity {report.minSeverity}</code>:
            {report.summary.issuesHidden}
            {report.summary.issuesHidden === 1 ? 'issue is' : 'issues are'} not included.
          </li>
        {/if}
        {#if report.issuesTruncated}
          <li>Showing the first {keptIssues(report)} of {report.issuesTotal} issues.</li>
        {/if}
        {#if report.errorsTruncated}
          <li>
            Showing the first {report.errors.length} of {report.errorsTotal} unreadable files.
          </li>
        {/if}
      </ul>
    {/if}

    <div class="toolbar">
      <nav class="picker" aria-label="Severity">
        {#each SEVERITY_FILTERS as option (option.id)}
          <a
            class="option"
            href={hrefWithQuery(['library'], { severity: option.id === 'all' ? null : option.id })}
            aria-current={option.id === filter ? 'true' : undefined}>{option.label}</a
          >
        {/each}
      </nav>
      {#if albums.length > 0}
        <div class="expand">
          <button type="button" onclick={expandAll}>Expand all</button>
          <button type="button" onclick={collapseAll}>Collapse all</button>
        </div>
      {/if}
    </div>

    {#if shownIssues === 0}
      <EmptyState
        title={keptIssues(report) === 0 ? 'No issues found' : 'No issues at this severity'}
      >
        {#if keptIssues(report) > 0}
          <a href={hrefWithQuery(['library'], {})}>Show all issues</a>
        {:else}
          Liner found nothing to report in this library.
        {/if}
      </EmptyState>
    {/if}

    {#if albums.length > 0}
      <Section title="Albums" count={albums.length}>
        <ul class="albums">
          {#each albums as album (album.id)}
            {@const counts = countBySeverity(album.issues)}
            <li>
              <details
                class="album"
                bind:open={
                  () => expanded.has(album.id),
                  (open) => (open ? expanded.add(album.id) : expanded.delete(album.id))
                }
              >
                <summary>
                  <span class="title">
                    <span class="album-title">{album.title ?? album.id}</span>
                    {#if album.albumArtist}<span class="artist">{album.albumArtist}</span>{/if}
                  </span>
                  <span class="counts">
                    {#each SEVERITY_ORDER as severity (severity)}
                      {#if counts[severity] > 0}
                        <SeverityChip
                          {severity}
                          label={severityCount(severity, counts[severity])}
                        />
                      {/if}
                    {/each}
                  </span>
                  {#if album.title !== null}<code class="id">{album.id}</code>{/if}
                </summary>
                <IssueList issues={album.issues} albumId={album.id} />
              </details>
            </li>
          {/each}
        </ul>
      </Section>
    {/if}

    {#if libraryIssues.length > 0}
      <Section title="Library-wide issues" count={libraryIssues.length}>
        <div class="panel"><IssueList issues={libraryIssues} /></div>
      </Section>
    {/if}

    {#if report.errors.length > 0}
      <Section title="Unreadable files" count={report.errorsTotal}>
        <ul class="panel errors">
          {#each report.errors as error, i (i)}
            <li>
              <code class="path">{error.path}</code>
              <span class="kind">{error.kind}</span>
              <p class="message">{error.message}</p>
            </li>
          {/each}
        </ul>
      </Section>
    {/if}
  {/if}
</div>

<style>
  .page {
    display: grid;
    gap: var(--space-5);
    min-width: 0;
  }

  .note {
    font-size: var(--text-sm);
    color: var(--color-text-muted);
  }

  .warn {
    color: var(--health-warning-fg);
  }

  .notes {
    display: grid;
    gap: var(--space-1);
    margin: 0;
    padding: var(--space-3) var(--space-4);
    list-style: none;
    background: var(--color-surface-sunken);
    border-radius: var(--radius-md);
  }

  .retry {
    padding: 0;
    color: var(--color-accent);
    background: none;
    border: 0;
    text-decoration: underline;
    cursor: pointer;
  }

  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 9rem), 1fr));
    gap: var(--space-3);
    margin: 0;
  }

  .tile {
    display: grid;
    gap: var(--space-1);
    padding: var(--space-3);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
  }

  .tile dt {
    font-size: var(--text-xs);
    color: var(--color-text-faint);
  }

  .tile dd {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: var(--weight-semibold);
    font-variant-numeric: tabular-nums;
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .picker {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 2px;
    padding: 2px;
    background: var(--color-surface-sunken);
    border-radius: var(--radius-md);
  }

  .option {
    padding: var(--space-1) var(--space-3);
    font-size: var(--text-sm);
    color: var(--color-text-muted);
    text-decoration: none;
    border-radius: var(--radius-sm);
  }

  .option:hover {
    color: var(--color-text);
  }

  .option[aria-current='true'] {
    font-weight: var(--weight-semibold);
    color: var(--color-text);
    background: var(--color-surface-raised);
    box-shadow: var(--shadow-sm);
  }

  .expand {
    display: flex;
    gap: var(--space-2);
  }

  .expand button {
    padding: var(--space-1) var(--space-3);
    font: inherit;
    font-size: var(--text-sm);
    color: var(--color-text);
    background: var(--color-surface);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }

  .expand button:hover {
    background: var(--color-surface-sunken);
  }

  .albums {
    display: grid;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .album,
  .panel {
    min-width: 0;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
  }

  .panel {
    margin: 0;
    padding: 0 var(--space-4);
  }

  .album > :global(.issues) {
    padding: 0 var(--space-4) var(--space-2) var(--space-6);
  }

  summary {
    display: grid;
    grid-template-columns: 1fr;
    gap: var(--space-1);
    padding: var(--space-3) var(--space-4) var(--space-3) var(--space-6);
    position: relative;
    cursor: pointer;
    list-style: none;
  }

  summary::-webkit-details-marker {
    display: none;
  }

  /*
   * A grid summary loses the native marker, so draw a chevron in the left padding (wrapped lines
   * stay aligned with the title); it turns down when the album is open.
   */
  summary::before {
    content: '';
    position: absolute;
    top: calc(var(--space-3) + 0.45em);
    left: var(--space-4);
    width: 0.45rem;
    height: 0.45rem;
    border-right: 2px solid var(--color-text-faint);
    border-bottom: 2px solid var(--color-text-faint);
    transform: rotate(-45deg);
    transition: transform var(--duration-fast);
  }

  .album[open] > summary::before {
    transform: rotate(45deg);
  }

  .title {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0 var(--space-2);
    min-width: 0;
  }

  .album-title {
    font-weight: var(--weight-medium);
    overflow-wrap: anywhere;
  }

  .artist {
    font-size: var(--text-sm);
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }

  .counts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .id {
    font-size: var(--text-xs);
    color: var(--color-text-faint);
    overflow-wrap: anywhere;
  }

  @media (min-width: 40rem) {
    summary {
      grid-template-columns: 1fr auto;
    }

    .counts {
      grid-column: 2;
      grid-row: 1;
      justify-content: flex-end;
    }
  }

  .errors {
    list-style: none;
  }

  .errors li {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-1) var(--space-2);
    padding-block: var(--space-3);
    border-top: 1px solid var(--color-border);
  }

  .errors li:first-child {
    border-top: none;
  }

  .path {
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
    min-width: 0;
  }

  .kind {
    font-size: var(--text-xs);
    color: var(--color-text-faint);
    text-transform: uppercase;
    letter-spacing: var(--tracking-wide);
  }

  .errors .message {
    flex-basis: 100%;
    font-size: var(--text-sm);
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }
</style>
