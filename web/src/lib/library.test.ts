import { describe, expect, it } from 'vitest';
import { fixtures } from './api/fixtures';
import type { LibraryIssue } from './api/types';
import {
  countBySeverity,
  displayPath,
  filterAlbums,
  filterIssues,
  morePaths,
  parseSeverityFilter,
  passes,
  severityCount,
  SEVERITY_ORDER,
} from './library';

const report = fixtures['library.json'].report!;

const issue = (
  severity: LibraryIssue['severity'],
  paths = 1,
  pathsTotal = paths,
): LibraryIssue => ({
  code: 'x',
  severity,
  scope: 'album',
  message: 'm',
  paths: Array.from({ length: paths }, (_, i) => `A/B/${i}.flac`),
  pathsTotal,
});

describe('parseSeverityFilter', () => {
  it.each([
    ['', 'all'],
    ['severity=warning', 'warning'],
    ['severity=error', 'error'],
    ['severity=all', 'all'],
    ['severity=info', 'all'],
    ['severity=bogus', 'all'],
  ] as const)('%s → %s', (query, expected) => {
    expect(parseSeverityFilter(new URLSearchParams(query))).toBe(expected);
  });
});

describe('passes', () => {
  it.each([
    ['all', ['error', 'warning', 'info']],
    ['warning', ['error', 'warning']],
    ['error', ['error']],
  ] as const)('%s keeps %j', (filter, kept) => {
    expect(SEVERITY_ORDER.filter((s) => passes(s, filter))).toEqual(kept);
  });
});

describe('filterIssues and filterAlbums', () => {
  it('keeps everything for "all"', () => {
    expect(filterAlbums(report.albums, 'all')).toEqual(report.albums);
    expect(filterIssues(report.libraryIssues, 'all')).toEqual(report.libraryIssues);
  });

  it('drops issues below the filter and albums left empty, keeping order', () => {
    const errors = filterAlbums(report.albums, 'error');
    expect(errors.map((a) => a.id)).toEqual([
      'Sample Person/Field Notes',
      'The Examples/Night Drive',
    ]);
    expect(errors.every((a) => a.issues.every((i) => i.severity === 'error'))).toBe(true);

    const warnings = filterAlbums(report.albums, 'warning');
    expect(warnings.map((a) => a.id)).toEqual([
      'Sample Person/Field Notes',
      'The Examples/Night Drive',
      'Various/Summer Mix',
    ]);
    expect(warnings[1]?.issues.map((i) => i.code)).toEqual(['likely-missing-disc-numbers']);
    expect(filterIssues(report.libraryIssues, 'error')).toEqual([]);
  });

  it('does not change the input', () => {
    const before = structuredClone(report.albums);
    filterAlbums(report.albums, 'error');
    expect(report.albums).toEqual(before);
  });
});

describe('countBySeverity', () => {
  it('counts every severity, including zeros', () => {
    expect(countBySeverity(report.albums[0]!.issues)).toEqual({ error: 1, warning: 2, info: 0 });
    expect(countBySeverity([])).toEqual({ error: 0, warning: 0, info: 0 });
  });
});

describe('displayPath', () => {
  it.each([
    ['Artist/Album/01 a.flac', 'Artist/Album', '01 a.flac'],
    ['Artist/Album/CD1/01 a.flac', 'Artist/Album', 'CD1/01 a.flac'],
    ['Artist/Album/01 a.flac', 'Artist/Album/', '01 a.flac'],
    // A sibling directory that shares the prefix is not inside the album.
    ['Artist/Album (Bonus)/01 a.flac', 'Artist/Album', 'Artist/Album (Bonus)/01 a.flac'],
    ['Artist/Albumx/01.flac', 'Artist/Album', 'Artist/Albumx/01.flac'],
    ['Other/01 a.flac', 'Artist/Album', 'Other/01 a.flac'],
    ['Artist/Album/', 'Artist/Album', 'Artist/Album/'],
    ['Artist/Album/01 a.flac', null, 'Artist/Album/01 a.flac'],
    ['01 a.flac', '', '01 a.flac'],
  ])('%s under %s → %s', (path, album, expected) => {
    expect(displayPath(path, album)).toBe(expected);
  });
});

describe('severityCount', () => {
  it.each([
    ['error', 1, '1 error'],
    ['error', 2, '2 errors'],
    ['warning', 0, '0 warnings'],
    ['warning', 1, '1 warning'],
    ['info', 1, '1 info'],
    ['info', 3, '3 info'],
  ] as const)('%s × %d → %s', (severity, count, expected) => {
    expect(severityCount(severity, count)).toBe(expected);
  });
});

describe('morePaths', () => {
  it('names the paths beyond the cap', () => {
    expect(morePaths(issue('info', 2, 20))).toBe('and 18 more');
    expect(morePaths(issue('info', 2, 2))).toBeNull();
    expect(morePaths(issue('info', 0, 0))).toBeNull();
  });
});
