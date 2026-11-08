// @vitest-environment jsdom
import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiRequestError } from '../lib/api/client';
import { fixtures } from '../lib/api/fixtures';
import type { LibraryResponse } from '../lib/api/types';
import type { SeverityFilter } from '../lib/library';
import type { Visibility } from '../lib/poller';
import LibraryPage from './LibraryPage.svelte';

const NOW = 1_794_200_000;
const visible: Visibility = { isHidden: () => false, subscribe: () => () => {} };
const example = fixtures['library.json'];

async function renderPage(
  load: () => Promise<LibraryResponse>,
  filter: SeverityFilter = 'all',
  name?: string,
) {
  const client = { library: vi.fn(load) };
  render(LibraryPage, { filter, client, visibility: visible, serverNow: NOW, name });
  await vi.advanceTimersByTimeAsync(0);
  return client;
}

function header(): HTMLElement {
  return screen.getByRole('region', { name: 'Music library summary' });
}

function albums(): HTMLDetailsElement[] {
  const section = screen.queryByRole('region', { name: /^Albums/ });
  return section ? [...section.querySelectorAll('details')] : [];
}

const text = (el: Element | null | undefined) => el?.textContent?.replace(/\s+/g, ' ').trim();

describe('LibraryPage', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW * 1000);
  });
  afterEach(() => vi.useRealTimers());

  it('shows the header, facts and summary tiles of a report', async () => {
    const client = await renderPage(async () => example);
    expect(client.library).toHaveBeenCalledTimes(1);
    expect(within(header()).getByRole('heading', { level: 1 }).textContent).toBe('Music library');
    expect(within(header()).getByText('Warning')).toBeTruthy();
    expect(header().textContent).toContain(
      '2 errors, 4 warnings, 1 info in 95 albums; 1 unreadable file',
    );
    const facts = text(header().querySelector('dl'));
    expect(facts).toContain('Report written 18h ago');
    expect(facts).toContain('Report tool liner 0.1.0 schema 1.0.0');
    expect(facts).toContain('Last checked 20s ago');

    const tiles = [
      ...screen.getByRole('region', { name: 'Report summary' }).querySelectorAll('dl > div'),
    ];
    expect(tiles.map(text)).toEqual([
      'Albums 95',
      'Audio files 1148',
      'Unreadable files 1',
      'Errors 2',
      'Warnings 4',
      'Info 1',
    ]);
  });

  it('polls every 60s', async () => {
    const client = await renderPage(async () => example);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(client.library).toHaveBeenCalledTimes(2);
  });

  it('groups issues per album, collapsed, with per-severity counts', async () => {
    await renderPage(async () => example);
    const rows = albums();
    expect(rows).toHaveLength(3);
    expect(rows.every((d) => !d.open)).toBe(true);
    const first = rows[0]!.querySelector('summary');
    // A null title falls back to the id; the id is not repeated then.
    expect(text(first)).toBe('Field Notes 1 error 2 warnings Sample Person/Field Notes');
    expect(text(rows[1]!.querySelector('summary'))).toBe(
      'Night Drive The Examples 1 error 1 info The Examples/Night Drive',
    );
  });

  it('renders issues with chip, code, message and album-relative paths', async () => {
    await renderPage(async () => example);
    const nightDrive = albums()[1]!;
    const info = [...nightDrive.querySelectorAll('.issue')][1]!;
    expect(info.querySelector('[data-severity]')?.getAttribute('data-severity')).toBe('info');
    expect(info.querySelector('[data-severity]')?.textContent?.trim()).toBe('Info');
    expect(info.querySelector('[data-severity] svg')?.getAttribute('aria-hidden')).toBe('true');
    expect(text(info.querySelector('.code'))).toBe('inconsistent-year');
    expect(text(info.querySelector('.message'))).toBe('Tracks have 2 different years: 2019, 2021');
    const files = within(info as HTMLElement).getByRole('list', { name: 'Files' });
    const paths = within(files)
      .getAllByRole('listitem')
      .map((li) => li.textContent);
    expect(paths).toEqual(['01 Ignition.flac', '01 Overture.flac', 'and 18 more']);
    // The full path is still available on hover.
    expect(files.querySelector('li')?.getAttribute('title')).toBe(
      'The Examples/Night Drive/01 Ignition.flac',
    );
  });

  it('never renders report text as HTML', async () => {
    const hostile = structuredClone(example);
    hostile.report!.albums[0]!.issues[0]!.message = '<img src=x onerror=alert(1)>';
    hostile.report!.albums[0]!.title = '<b>bold</b>';
    hostile.report!.errors[0]!.path = '<script>x</script>.flac';
    await renderPage(async () => hostile);
    expect(document.querySelector('img, b, script')).toBeNull();
    expect(document.body.textContent).toContain('<img src=x onerror=alert(1)>');
    expect(document.body.textContent).toContain('<b>bold</b>');
  });

  it('expands and collapses every album', async () => {
    await renderPage(async () => example);
    await fireEvent.click(screen.getByRole('button', { name: 'Expand all' }));
    expect(albums().every((d) => d.open)).toBe(true);
    await fireEvent.click(screen.getByRole('button', { name: 'Collapse all' }));
    expect(albums().every((d) => !d.open)).toBe(true);
  });

  // KNOWN_ISSUES (M7.4): an album closed just before "Expand all" stayed closed. Its `toggle`
  // event is async, so the set still held it when "Expand all" ran (adding it changed nothing),
  // and the late event then removed it.
  it('expands an album whose close is still pending', async () => {
    await renderPage(async () => example);
    const first = albums()[0]!;
    first.open = true;
    await vi.advanceTimersByTimeAsync(0);
    first.open = false;
    await fireEvent.click(screen.getByRole('button', { name: 'Expand all' }));
    await vi.advanceTimersByTimeAsync(0);
    expect(albums().map((d) => d.open)).toEqual(albums().map(() => true));

    // And the mirror case: an album opened just before "Collapse all".
    await fireEvent.click(screen.getByRole('button', { name: 'Collapse all' }));
    await vi.advanceTimersByTimeAsync(0);
    first.open = true;
    await fireEvent.click(screen.getByRole('button', { name: 'Collapse all' }));
    await vi.advanceTimersByTimeAsync(0);
    expect(albums().some((d) => d.open)).toBe(false);
  });

  it('keeps an album the user opened open across a poll', async () => {
    await renderPage(async () => example);
    const first = albums()[0]!;
    first.open = true;
    await fireEvent(first, new Event('toggle'));
    await vi.advanceTimersByTimeAsync(60_000);
    expect(albums()[0]!.open).toBe(true);
    expect(albums()[1]!.open).toBe(false);
  });

  it('lists library-wide issues and unreadable files in their own sections', async () => {
    await renderPage(async () => example);
    const library = screen.getByRole('region', { name: /^Library-wide issues/ });
    expect(library.textContent).toContain('split-album');
    // Library-scope paths are shown in full: there is no album to be relative to.
    expect(library.textContent).toContain('Quiet Hours/Live (Bonus)/01 Encore.flac');
    const errors = screen.getByRole('region', { name: /^Unreadable files/ });
    expect(text(errors)).toContain(
      'Sample Person/Field Notes/05 Unfinished.flac corrupt failed to parse Flac file: failed to fill whole buffer',
    );
  });

  it('offers a severity filter as links, marking the current one', async () => {
    await renderPage(async () => example, 'warning');
    const nav = screen.getByRole('navigation', { name: 'Severity' });
    const links = within(nav).getAllByRole('link');
    expect(links.map((a) => [a.textContent, a.getAttribute('href')])).toEqual([
      ['All', '#/library'],
      ['Warnings and errors', '#/library?severity=warning'],
      ['Errors only', '#/library?severity=error'],
    ]);
    expect(links[1]?.getAttribute('aria-current')).toBe('true');
    expect(links[0]?.getAttribute('aria-current')).toBeNull();
  });

  it('filters issues by severity', async () => {
    await renderPage(async () => example, 'error');
    const rows = albums();
    expect(rows).toHaveLength(2);
    expect(document.querySelectorAll('.issue')).toHaveLength(2);
    expect(screen.getByRole('region', { name: /^Albums/ }).textContent).not.toContain('warning');
    // The split-album warning is filtered out, so its section goes away.
    expect(screen.queryByRole('region', { name: /^Library-wide issues/ })).toBeNull();
    // Unreadable files are not issues and stay listed.
    expect(screen.getByRole('region', { name: /^Unreadable files/ })).toBeTruthy();
  });

  it('says when a filter leaves nothing, with a way back', async () => {
    const onlyInfo = structuredClone(example);
    onlyInfo.report!.albums = [onlyInfo.report!.albums[1]!];
    onlyInfo.report!.albums[0]!.issues = [onlyInfo.report!.albums[0]!.issues[1]!];
    onlyInfo.report!.libraryIssues = [];
    await renderPage(async () => onlyInfo, 'error');
    expect(screen.getByText('No issues at this severity')).toBeTruthy();
    expect(screen.getByRole('link', { name: 'Show all issues' }).getAttribute('href')).toBe(
      '#/library',
    );
    expect(screen.queryByRole('button', { name: 'Expand all' })).toBeNull();
  });

  it('says when the report has no issues', async () => {
    const clean = structuredClone(example);
    clean.report!.albums = [];
    clean.report!.libraryIssues = [];
    clean.report!.errors = [];
    await renderPage(async () => clean);
    expect(screen.getByText('No issues found')).toBeTruthy();
    expect(screen.queryByRole('region', { name: /^Unreadable files/ })).toBeNull();
  });

  it('notes truncation and issues hidden by --min-severity', async () => {
    const capped = structuredClone(example);
    capped.report!.minSeverity = 'warning';
    capped.report!.summary.issuesHidden = 12;
    capped.report!.issuesTotal = 900;
    capped.report!.issuesTruncated = true;
    capped.report!.errorsTotal = 250;
    capped.report!.errorsTruncated = true;
    await renderPage(async () => capped);
    const body = text(document.body);
    expect(body).toContain('written with --min-severity warning: 12 issues are not included');
    expect(body).toContain('Showing the first 7 of 900 issues.');
    expect(body).toContain('Showing the first 1 of 250 unreadable files.');
    expect(screen.getByRole('region', { name: /^Unreadable files/ }).textContent).toContain('250');
  });

  it('has no notes for a complete report', async () => {
    await renderPage(async () => example);
    expect(text(document.body)).not.toMatch(/Showing the first|min-severity/);
  });

  it('shows the reason and no tables when there is no valid report', async () => {
    const broken: LibraryResponse = {
      ...structuredClone(example),
      library: {
        ...example.library!,
        health: 'unknown',
        message: 'unsupported Liner schema 2.0.0',
        generatedAt: null,
        age: null,
        issueCounts: null,
        unreadableFiles: null,
        albums: null,
      },
      report: null,
    };
    await renderPage(async () => broken);
    expect(within(header()).getByText('Unknown')).toBeTruthy();
    expect(header().textContent).toContain('unsupported Liner schema 2.0.0');
    expect(text(header().querySelector('dl'))).toContain('Report written No report');
    expect(screen.getByText('No report to show')).toBeTruthy();
    expect(screen.queryByRole('region', { name: 'Report summary' })).toBeNull();
    expect(screen.queryByRole('navigation', { name: 'Severity' })).toBeNull();
    expect(albums()).toHaveLength(0);
  });

  it('points to the config when the library is not configured', async () => {
    await renderPage(async () => fixtures['library-unconfigured.json']);
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Music library');
    expect(screen.getByText('No music library configured')).toBeTruthy();
    expect(document.body.textContent).toContain('[library]');
    expect(document.body.textContent).toContain('docs/CONFIG.md');
    expect(screen.queryByRole('navigation', { name: 'Severity' })).toBeNull();
  });

  it('shows a loading state with the known name as heading', async () => {
    await renderPage(() => new Promise(() => {}), 'all', 'Records');
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Records');
    expect(screen.getByRole('status', { name: 'Loading library' })).toBeTruthy();
    expect(screen.getByRole('status', { name: 'Loading library issues' })).toBeTruthy();
  });

  it('shows an error with retry when offline before the first response', async () => {
    const client = await renderPage(async () => {
      throw new ApiRequestError('network', 'Failed to fetch', 0);
    });
    expect(screen.getByText('Could not load the library report')).toBeTruthy();
    client.library.mockResolvedValue(example);
    await fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    await vi.advanceTimersByTimeAsync(0);
    expect(albums()).toHaveLength(3);
  });

  it('keeps the last report with a note when a refresh fails', async () => {
    const client = await renderPage(async () => example);
    client.library.mockRejectedValue(new ApiRequestError('network', 'Failed to fetch', 0));
    await vi.advanceTimersByTimeAsync(60_000);
    expect(screen.getByRole('alert').textContent).toContain('Refresh failed');
    expect(albums()).toHaveLength(3);
  });
});
