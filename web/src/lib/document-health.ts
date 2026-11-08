// The browser tab mirrors the overall health, so a problem is visible from another tab: the
// title leads with the worst state and how many targets are in it, and the favicon's eyes and
// corner dot take that state's color. Both stay quiet while everything is healthy.

import type { Health, Overview } from './api/types';

export const APP_NAME = 'Gargoyle';

/** Leading symbol per state; text follows it, so the symbol is never the only signal. */
const TITLE_SYMBOL: Record<Health, string> = {
  healthy: '',
  warning: '⚠',
  critical: '✖',
  unknown: '?',
};

/** "2 warnings", "1 critical", "3 unknown". */
function countLabel(health: Exclude<Health, 'healthy'>, count: number): string {
  if (health === 'warning') return `${count} ${count === 1 ? 'warning' : 'warnings'}`;
  return `${count} ${health}`;
}

/**
 * `Gargoyle — host` when healthy, `⚠ 2 warnings · Gargoyle — host` otherwise, `Offline · …` when
 * the last poll failed (the shown health may be out of date) or `Sign-in needed · …` on a 401, and plain `Gargoyle` before the
 * first response. A page other than the overview names itself before the app:
 * `⚠ 2 warnings · System · Gargoyle — host`, so tabs and history entries are told apart.
 */
/** Why the shown health may be out of date: the server is unreachable, or it refuses us (401). */
export type FeedProblem = 'offline' | 'unauthorized';

const PROBLEM_PREFIX: Record<FeedProblem, string> = {
  offline: 'Offline',
  unauthorized: 'Sign-in needed',
};

export function documentTitle(
  overview: Overview | null,
  problem: FeedProblem | null,
  page: string | null = null,
): string {
  const app = overview === null ? APP_NAME : `${APP_NAME} — ${overview.host}`;
  const base = page === null ? app : `${page} · ${app}`;
  if (problem !== null) return `${PROBLEM_PREFIX[problem]} · ${base}`;
  if (overview === null) return base;
  const health = overview.health;
  if (health === 'healthy') return base;
  // `counts` can lag the aggregate in theory (the system is one target); never print "0 ...".
  const count = Math.max(1, overview.counts[health]);
  return `${TITLE_SYMBOL[health]} ${countLabel(health, count)} · ${base}`;
}

/**
 * Favicon colors. Fixed values (a data URI cannot read CSS variables), taken from the dark theme's
 * `--health-*-fg` in tokens.css: they hold up on both light and dark browser tab strips.
 */
const FAVICON_COLOR: Record<Health, string> = {
  healthy: '#6db5a6', // the mark's verdigris eyes, as in index.html
  warning: '#e3ad55',
  critical: '#ef8173',
  unknown: '#aaa398',
};

const STONE = '#736d63';

/** The gargoyle mark (horned stone shield) with eyes and, unless healthy, a status dot. */
export function faviconSvg(health: Health | null): string {
  const eyes = FAVICON_COLOR[health ?? 'healthy'];
  const dot =
    health === null || health === 'healthy'
      ? ''
      : `<circle cx='25' cy='25' r='6.5' fill='${FAVICON_COLOR[health]}' stroke='#fff' stroke-width='1.5'/>`;
  return (
    "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'>" +
    `<path d='M6 3l4.5 5h11L26 3v13c0 7-4.6 11.2-10 13.5C10.6 27.2 6 23 6 16z' fill='${STONE}'/>` +
    `<circle cx='12.5' cy='15.5' r='2' fill='${eyes}'/>` +
    `<circle cx='19.5' cy='15.5' r='2' fill='${eyes}'/>` +
    dot +
    '</svg>'
  ).replaceAll('#', '%23');
}

/** A `data:` URI for `<link rel="icon">`; allowed by the CSP's `img-src 'self' data:`. */
export function faviconHref(health: Health | null): string {
  return `data:image/svg+xml,${faviconSvg(health)}`;
}

/** Points the page's icon link at `href`, adding the link if index.html lacks one. */
export function setFavicon(doc: Document, href: string): void {
  let link = doc.querySelector<HTMLLinkElement>('link[rel="icon"]');
  if (!link) {
    link = doc.createElement('link');
    link.rel = 'icon';
    link.type = 'image/svg+xml';
    doc.head.append(link);
  }
  if (link.getAttribute('href') !== href) link.setAttribute('href', href);
}
