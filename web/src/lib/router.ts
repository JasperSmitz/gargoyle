// Hash routing (`/#/services/nginx.service`): the backend serves only `/`, so no server-side
// fallback is needed (ARCHITECTURE §6). Pages are added by extending ROUTES and App's page switch.

import type { TargetKind } from './api/types';

const ROUTES = [
  { name: 'overview', pattern: '/' },
  { name: 'system', pattern: '/system' },
  { name: 'events', pattern: '/events' },
  { name: 'library', pattern: '/library' },
  { name: 'service', pattern: '/services/:unit' },
  { name: 'timer', pattern: '/timers/:unit' },
  { name: 'check', pattern: '/checks/:name' },
] as const satisfies readonly {
  name: string;
  pattern: string;
}[];

export type RouteName = (typeof ROUTES)[number]['name'] | 'not-found';

export interface Route {
  name: RouteName;
  /** Decoded `:param` values. */
  params: Record<string, string>;
  /** The path that was matched, e.g. `/services/nginx.service`. */
  path: string;
}

/** Matches `/a/:b` against `/a/x%20y`, returning `{ b: 'x y' }`, or null on mismatch. */
export function matchPattern(pattern: string, path: string): Record<string, string> | null {
  const want = segments(pattern);
  const have = segments(path);
  if (want.length !== have.length) return null;
  const params: Record<string, string> = {};
  for (const [i, part] of want.entries()) {
    const actual = have[i] ?? '';
    if (part.startsWith(':')) {
      try {
        params[part.slice(1)] = decodeURIComponent(actual);
      } catch {
        return null; // malformed escape such as `%E0`
      }
    } else if (part !== actual) {
      return null;
    }
  }
  return params;
}

/** `#/x/y?z` → route for `/x/y`. An empty hash or `#` is the overview. */
export function parseHash(hash: string): Route {
  const path = '/' + segments(hash.replace(/^#/, '').split('?')[0] ?? '').join('/');
  for (const route of ROUTES) {
    const params = matchPattern(route.pattern, path);
    if (params) return { name: route.name, params, path };
  }
  return { name: 'not-found', params: {}, path };
}

/** Builds a link target, encoding each dynamic segment: `href('services', unit)`. */
export function href(...parts: string[]): string {
  return '#/' + parts.map(encodeURIComponent).join('/');
}

/** Query parameters after the path, `#/system?range=7d` → `{ range: '7d' }`. */
export function hashQuery(hash: string): URLSearchParams {
  const query = hash.indexOf('?');
  return new URLSearchParams(query === -1 ? '' : hash.slice(query + 1));
}

/** `href(...)` plus a query string; unset values are left out. */
export function hrefWithQuery(
  parts: string[],
  query: Record<string, string | null | undefined>,
): string {
  const params = new URLSearchParams();
  for (const [name, value] of Object.entries(query)) {
    if (value !== null && value !== undefined) params.set(name, value);
  }
  const encoded = params.toString();
  return href(...parts) + (encoded ? `?${encoded}` : '');
}

/**
 * Where a target is shown in detail. System targets (`cpu`, `disk:/`) all live on the System page;
 * the library is a single target (`library`) with its own page.
 */
export function targetHref(kind: TargetKind, target: string): string {
  switch (kind) {
    case 'service':
      return href('services', target);
    case 'timer':
      return href('timers', target);
    case 'system':
      return href('system');
    case 'check':
      return href('checks', target);
    case 'library':
      return href('library');
  }
}

function segments(path: string): string[] {
  return path.split('/').filter((s) => s !== '');
}
