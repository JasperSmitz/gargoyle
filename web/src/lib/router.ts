// Hash routing (`/#/services/nginx.service`): the backend serves only `/`, so no server-side
// fallback is needed (ARCHITECTURE §6). Pages are added by extending ROUTES and App's page switch,
// e.g. `{ name: 'service', pattern: '/services/:unit' }`.

const ROUTES = [{ name: 'overview', pattern: '/' }] as const satisfies readonly {
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

function segments(path: string): string[] {
  return path.split('/').filter((s) => s !== '');
}
