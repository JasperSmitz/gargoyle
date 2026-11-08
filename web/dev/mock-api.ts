// Dev-only stand-in for the backend: `VITE_MOCK=1 pnpm dev` serves the contract examples
// (docs/api/examples) from the Vite dev server instead of proxying to 127.0.0.1:9077.
// vite.config.ts loads this module with `ssrLoadModule` from a `serve`-only plugin, so nothing
// here can reach the production bundle (or the config loader).
//
// Metrics are generated (see mock-metrics.ts) for whatever window is asked, so the System page
// works for every range. Timers, runs and events are generated too (mock-units.ts), so detail
// pages, countdowns and event paging work. The overview is lightly animated (fresh timestamps,
// wandering CPU and memory) so polling and "updated Xs ago" can be seen working.
// Checks come from mock-checks.ts, one per health state.
// `VITE_MOCK=offline` answers 503 everywhere instead, to see the offline banner, and
// `VITE_MOCK=empty` serves a host with no services, timers, checks or library configured.
// The library report is the contract example, its times moved to read the same relative to now.

import { fixtures } from '../src/lib/api/fixtures';
import {
  TARGET_KINDS,
  type Health,
  type HealthCounts,
  type LibraryResponse,
  type LibrarySummary,
  type Overview,
  type ServiceSummary,
} from '../src/lib/api/types';
import { worst } from '../src/lib/health';
import { check, checks } from './mock-checks';
import { QueryError, query, seriesInfo } from './mock-metrics';
import { events, timerDetail, timers } from './mock-units';

type Fixture = keyof typeof fixtures;

const HOST = 'debian-box';

const ROUTES: [RegExp, Fixture][] = [[/^\/health$/, 'health.json']];

/** `normal` serves generated data, `offline` fails every call, `empty` has no targets configured. */
export type MockMode = 'normal' | 'offline' | 'empty';

export interface MockResponse {
  status: number;
  body: unknown;
}

/** Answers `GET /api/v1<path>?<params>`. */
export function respond(
  path: string,
  params: URLSearchParams,
  mode: MockMode = 'normal',
  now = Date.now() / 1000,
): MockResponse {
  if (mode === 'offline') {
    return {
      status: 503,
      body: { error: { code: 'unavailable', message: 'mock backend is offline' } },
    };
  }
  if (path === '/overview') {
    return { status: 200, body: mode === 'empty' ? emptyOverview(now) : liveOverview(now) };
  }
  if (path === '/library') {
    return {
      status: 200,
      body: mode === 'empty' ? fixtures['library-unconfigured.json'] : liveLibrary(now),
    };
  }
  if (path === '/metrics') return { status: 200, body: { host: HOST, series: seriesInfo() } };
  if (path === '/metrics/query') {
    try {
      return { status: 200, body: query(params, now) };
    } catch (error) {
      if (!(error instanceof QueryError)) throw error;
      return { status: 400, body: { error: { code: error.code, message: error.message } } };
    }
  }
  const units = unitsResponse(path, params, now) ?? checksResponse(path, now);
  if (units) return units;
  const match = ROUTES.find(([pattern]) => pattern.test(path));
  if (match) return { status: 200, body: fixtures[match[1]] };
  return { status: 404, body: { error: { code: 'not_found', message: `no mock for ${path}` } } };
}

function notFound(what: string): MockResponse {
  return { status: 404, body: { error: { code: 'not_found', message: `unknown ${what}` } } };
}

/** `/services…`, `/timers…` and `/events`, or null for other paths. */
function unitsResponse(path: string, params: URLSearchParams, now: number): MockResponse | null {
  const t = Math.floor(now);
  const services = liveServices(t);
  if (path === '/services') return { status: 200, body: { host: HOST, services } };
  if (path === '/timers') return { status: 200, body: { host: HOST, timers: timers(t) } };
  if (path === '/events') {
    const body = {
      host: HOST,
      events: events(t, services, {
        since: intParam(params, 'since'),
        until: intParam(params, 'until'),
        target: params.get('target') ?? undefined,
        kind: TARGET_KINDS.find((k) => k === params.get('kind')),
        limit: intParam(params, 'limit'),
      }),
    };
    return { status: 200, body };
  }
  const [, collection, encoded] = /^\/(services|timers)\/([^/]+)$/.exec(path) ?? [];
  if (!collection || !encoded) return null;
  const unit = decodeURIComponent(encoded);
  const unitEvents = events(t, services, { target: unit, limit: 20 });
  if (collection === 'services') {
    const service = services.find((s) => s.unit === unit);
    if (!service) return notFound('service');
    return { status: 200, body: { host: HOST, service, events: unitEvents } };
  }
  const detail = timerDetail(unit, t, intParam(params, 'limit') ?? 50);
  if (!detail) return notFound('timer');
  return { status: 200, body: { host: HOST, ...detail, events: unitEvents } };
}

/** `/checks` and `/checks/{name}`, or null for other paths. */
function checksResponse(path: string, now: number): MockResponse | null {
  const t = Math.floor(now);
  if (path === '/checks') return { status: 200, body: { host: HOST, checks: checks(t) } };
  const [, encoded] = /^\/checks\/([^/]+)$/.exec(path) ?? [];
  if (!encoded) return null;
  const name = decodeURIComponent(encoded);
  const summary = check(name, t);
  if (!summary) return notFound('check');
  const checkEvents = events(t, liveServices(t), { target: name, kind: 'check', limit: 20 });
  return { status: 200, body: { host: HOST, check: summary, events: checkEvents } };
}

function intParam(params: URLSearchParams, name: string): number | undefined {
  const value = params.get(name);
  return value !== null && /^\d+$/.test(value) ? Number(value) : undefined;
}

/** The example's services, with times moved so they read the same relative to `now`. */
function liveServices(now: number): ServiceSummary[] {
  const shift = now - fixtures['overview.json'].ts;
  return fixtures['overview.json'].services.map((s) => ({
    ...s,
    since: s.since === null ? null : s.since + shift,
  }));
}

function liveOverview(at: number): Overview {
  const overview = structuredClone(fixtures['overview.json']);
  const now = Math.floor(at);
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

  // Keep relative times ("2h ago") as they read in the example.
  overview.services = liveServices(now);
  overview.checks = checks(now);
  overview.timers = timers(now);
  overview.recentEvents = events(now, overview.services, { limit: 10 });
  overview.library = liveLibrarySummary(now);

  // The generated timers differ from the example's, so health and counts are recomputed.
  const targets: Health[] = [
    overview.system.health,
    ...[...overview.services, ...overview.timers, ...overview.checks].map((t) => t.health),
    ...(overview.library ? [overview.library.health] : []),
  ];
  const counts: HealthCounts = { healthy: 0, warning: 0, critical: 0, unknown: 0 };
  for (const health of targets) counts[health] += 1;
  overview.counts = counts;
  overview.health = worst(targets);
  return overview;
}

/** A fresh install: only the system is watched, all is well, and nothing has happened yet. */
function emptyOverview(at: number): Overview {
  const overview = liveOverview(at);
  for (const disk of overview.system.disks) {
    disk.used = Math.round(disk.total * 0.3);
    disk.usedPct = 30;
    disk.health = 'healthy';
  }
  overview.system.health = 'healthy';
  overview.services = [];
  overview.timers = [];
  overview.checks = [];
  overview.library = null;
  overview.recentEvents = [];
  overview.health = overview.system.health;
  overview.counts = { healthy: 0, warning: 0, critical: 0, unknown: 0 };
  overview.counts[overview.system.health] = 1;
  return overview;
}

/** The example library summary, polled 20s ago, with a report as old as in the example. */
function liveLibrarySummary(now: number): LibrarySummary {
  const t = Math.floor(now);
  const example = fixtures['library.json'].library!;
  const age = example.age ?? 0;
  return { ...example, checkedAt: t - 20, generatedAt: t - age, age };
}

function liveLibrary(now: number): LibraryResponse {
  return { ...structuredClone(fixtures['library.json']), library: liveLibrarySummary(now) };
}

function round(value: number, digits = 1): number {
  return Number(value.toFixed(digits));
}
