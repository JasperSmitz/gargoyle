// Dev-only stand-in for the backend: `VITE_MOCK=1 pnpm dev` serves the contract examples
// (docs/api/examples) from the Vite dev server instead of proxying to 127.0.0.1:9077.
// vite.config.ts loads this module with `ssrLoadModule` from a `serve`-only plugin, so nothing
// here can reach the production bundle (or the config loader).
//
// Metrics are generated (see mock-metrics.ts) for whatever window is asked, so the System page
// works for every range. The overview is lightly animated (fresh timestamps, wandering CPU and memory) so polling and
// "updated Xs ago" can be seen working. `VITE_MOCK=offline` answers 503 everywhere instead, to
// see the offline banner.

import { fixtures } from '../src/lib/api/fixtures';
import type { Overview } from '../src/lib/api/types';
import { QueryError, query, seriesInfo } from './mock-metrics';

type Fixture = keyof typeof fixtures;

const ROUTES: [RegExp, Fixture][] = [
  [/^\/health$/, 'health.json'],
  [/^\/services$/, 'services.json'],
  [/^\/services\/[^/]+$/, 'service-detail.json'],
  [/^\/timers$/, 'timers.json'],
  [/^\/timers\/[^/]+$/, 'timer-detail.json'],
  [/^\/checks$/, 'checks.json'],
  [/^\/checks\/[^/]+$/, 'check-detail.json'],
  [/^\/events$/, 'events.json'],
];

export interface MockResponse {
  status: number;
  body: unknown;
}

/** Answers `GET /api/v1<path>?<params>`. */
export function respond(
  path: string,
  params: URLSearchParams,
  offline: boolean,
  now = Date.now() / 1000,
): MockResponse {
  if (offline) {
    return {
      status: 503,
      body: { error: { code: 'unavailable', message: 'mock backend is offline' } },
    };
  }
  if (path === '/overview') return { status: 200, body: liveOverview() };
  if (path === '/metrics')
    return { status: 200, body: { host: 'debian-box', series: seriesInfo() } };
  if (path === '/metrics/query') {
    try {
      return { status: 200, body: query(params, now) };
    } catch (error) {
      if (!(error instanceof QueryError)) throw error;
      return { status: 400, body: { error: { code: 'bad_request', message: error.message } } };
    }
  }
  const match = ROUTES.find(([pattern]) => pattern.test(path));
  if (match) return { status: 200, body: fixtures[match[1]] };
  return { status: 404, body: { error: { code: 'not_found', message: `no mock for ${path}` } } };
}

function liveOverview(): Overview {
  const overview = structuredClone(fixtures['overview.json']);
  const now = Math.floor(Date.now() / 1000);
  const shift = now - overview.ts;
  const wave = Math.sin(now / 40);

  overview.ts = now;
  overview.system.ts = now - 5;
  overview.hostInfo.uptime = now - overview.hostInfo.bootTime;
  overview.system.cpu.usage = round(14 + 10 * wave + Math.random() * 4);
  overview.system.cpu.iowait = round(0.3 + Math.random() * 0.6);
  overview.system.load.one = round(0.4 + 0.3 * wave, 2);
  const memory = overview.system.memory;
  memory.used = Math.round(memory.total * (0.26 + 0.03 * wave));
  memory.usedPct = round((memory.used / memory.total) * 100);
  for (const net of overview.system.net) {
    net.rxBps = Math.round(net.rxBps * (1 + 0.4 * wave + Math.random() * 0.3));
    net.txBps = Math.round(net.txBps * (1 + 0.3 * wave + Math.random() * 0.3));
  }

  // Keep relative times ("2h ago", "next in 21h") as they read in the example.
  for (const s of overview.services) if (s.since !== null) s.since += shift;
  for (const t of overview.timers) {
    if (t.nextRun !== null) t.nextRun += shift;
    if (t.lastRun) {
      t.lastRun.startedAt += shift;
      if (t.lastRun.finishedAt !== null) t.lastRun.finishedAt += shift;
    }
  }
  for (const c of overview.checks) if (c.lastCheck !== null) c.lastCheck += shift;
  for (const e of overview.recentEvents) e.ts += shift;
  return overview;
}

function round(value: number, digits = 1): number {
  return Number(value.toFixed(digits));
}
