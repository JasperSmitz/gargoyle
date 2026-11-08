// Presentation helpers for the music library page (`GET /api/v1/library`, ADR 0007). The backend
// already groups, orders and caps the report; the UI only filters by severity and shortens paths.

import {
  SEVERITIES,
  type LibraryAlbum,
  type LibraryIssue,
  type Severity,
  type SeverityCounts,
} from './api/types';

/** Which issues the page shows, kept in the URL as `?severity=` (absent means all). */
export const SEVERITY_FILTERS = [
  { id: 'all', label: 'All', min: 'info' },
  { id: 'warning', label: 'Warnings and errors', min: 'warning' },
  { id: 'error', label: 'Errors only', min: 'error' },
] as const satisfies readonly { id: string; label: string; min: Severity }[];

export type SeverityFilter = (typeof SEVERITY_FILTERS)[number]['id'];

export function parseSeverityFilter(query: URLSearchParams): SeverityFilter {
  const value = query.get('severity');
  return SEVERITY_FILTERS.find((f) => f.id === value && f.id !== 'all')?.id ?? 'all';
}

/** Error is the most severe; matches the backend's ordering. */
const RANK: Record<Severity, number> = { error: 2, warning: 1, info: 0 };

export interface SeverityPresentation {
  label: string;
  /** Distinct silhouettes, so a chip reads without color. */
  icon: 'octagon' | 'alert' | 'info';
}

export const SEVERITY_PRESENTATION: Record<Severity, SeverityPresentation> = {
  error: { label: 'Error', icon: 'octagon' },
  warning: { label: 'Warning', icon: 'alert' },
  info: { label: 'Info', icon: 'info' },
};

export function passes(severity: Severity, filter: SeverityFilter): boolean {
  const min = SEVERITY_FILTERS.find((f) => f.id === filter)?.min ?? 'info';
  return RANK[severity] >= RANK[min];
}

export function filterIssues(
  issues: readonly LibraryIssue[],
  filter: SeverityFilter,
): LibraryIssue[] {
  return issues.filter((issue) => passes(issue.severity, filter));
}

/** Albums with their issues filtered; albums left without issues are dropped, order kept. */
export function filterAlbums(
  albums: readonly LibraryAlbum[],
  filter: SeverityFilter,
): LibraryAlbum[] {
  if (filter === 'all') return [...albums];
  return albums.flatMap((album) => {
    const issues = filterIssues(album.issues, filter);
    return issues.length > 0 ? [{ ...album, issues }] : [];
  });
}

export function countBySeverity(issues: readonly LibraryIssue[]): SeverityCounts {
  const counts: SeverityCounts = { error: 0, warning: 0, info: 0 };
  for (const issue of issues) counts[issue.severity] += 1;
  return counts;
}

/** Severities from worst to least, for chips and summaries. */
export const SEVERITY_ORDER: readonly Severity[] = [...SEVERITIES].sort(
  (a, b) => RANK[b] - RANK[a],
);

/**
 * A path as shown under its album: relative to the album directory when it lies inside it
 * (`Artist/Album/01 x.flac` → `01 x.flac`), else unchanged. Paths are `/`-separated and relative
 * to the library root, as is the album id.
 */
export function displayPath(path: string, albumId: string | null): string {
  if (albumId === null || albumId === '') return path;
  const prefix = albumId.endsWith('/') ? albumId : `${albumId}/`;
  return path.startsWith(prefix) && path.length > prefix.length ? path.slice(prefix.length) : path;
}

/** "2 errors", "1 warning", "3 info" (info has no plural). */
export function severityCount(severity: Severity, count: number): string {
  if (severity === 'info') return `${count} info`;
  return `${count} ${severity}${count === 1 ? '' : 's'}`;
}

/** "and 18 more" for an issue whose paths were capped, or null when all are listed. */
export function morePaths(issue: LibraryIssue): string | null {
  const more = issue.pathsTotal - issue.paths.length;
  return more > 0 ? `and ${more} more` : null;
}
