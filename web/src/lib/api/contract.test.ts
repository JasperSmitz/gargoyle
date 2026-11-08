import { describe, expect, it } from 'vitest';
import type { Conforms } from './contract';
import { fixtures } from './fixtures';
import {
  CHECK_KINDS,
  HEALTH_STATES,
  ISSUE_SCOPES,
  RESOLUTIONS,
  RUN_SOURCES,
  SERIES_UNITS,
  SEVERITIES,
  TARGET_KINDS,
  WARN_ON,
  type CheckSummary,
  type HealthEvent,
  type ServiceSummary,
  type TimerSummary,
} from './types';

// Files only; nothing is loaded. Ensures a new example file cannot be added without a fixture.
const exampleFiles = Object.keys(import.meta.glob('../../../../docs/api/examples/*.json')).map(
  (path) => path.split('/').pop(),
);

/** Every value stored under `key` anywhere inside `value`. */
function valuesOf(value: unknown, key: string): unknown[] {
  if (Array.isArray(value)) return value.flatMap((item) => valuesOf(item, key));
  if (typeof value !== 'object' || value === null) return [];
  return Object.entries(value).flatMap(([k, v]) => (k === key ? [v] : []).concat(valuesOf(v, key)));
}

const all = Object.values(fixtures);

describe('contract examples', () => {
  it('has a typed fixture for every example file', () => {
    expect(exampleFiles.length).toBeGreaterThan(0);
    expect(Object.keys(fixtures).sort()).toEqual([...exampleFiles].sort());
  });

  it.each([
    ['health', HEALTH_STATES],
    ['targetKind', TARGET_KINDS],
    ['source', RUN_SOURCES],
    ['resolution', RESOLUTIONS],
    ['severity', SEVERITIES],
    ['minSeverity', SEVERITIES],
    ['scope', ISSUE_SCOPES],
    ['warnOn', WARN_ON],
  ] as const)('uses only known values for "%s"', (key, allowed) => {
    const values = valuesOf(all, key);
    expect(values.length).toBeGreaterThan(0);
    for (const value of values) expect(allowed).toContain(value);
  });

  // `kind` is closed for checks only; a library error `kind` (`corrupt`, ...) is an open set.
  it('uses only check kinds for checks', () => {
    const checks = [...valuesOf(all, 'checks'), ...valuesOf(all, 'check')].flat();
    const kinds = valuesOf(checks, 'kind');
    expect(kinds.length).toBeGreaterThan(0);
    for (const kind of kinds) expect(CHECK_KINDS).toContain(kind);
  });

  // `from`/`to` also name the metrics-query time range, so only look inside event lists.
  it('uses only health states for event transitions', () => {
    const events = [...valuesOf(all, 'events'), ...valuesOf(all, 'recentEvents')].flat();
    expect(events.length).toBeGreaterThan(0);
    for (const value of valuesOf(events, 'from')) {
      if (value !== null) expect(HEALTH_STATES).toContain(value);
    }
    for (const value of valuesOf(events, 'to')) expect(HEALTH_STATES).toContain(value);
  });

  it('uses only known series units', () => {
    const units = [
      ...fixtures['metrics.json'].series,
      ...fixtures['metrics-query.json'].series,
    ].map((s) => s.unit);
    for (const unit of units) expect(SERIES_UNITS).toContain(unit);
  });

  it('uses integer unix seconds for timestamps', () => {
    const keys = [
      'ts',
      'since',
      'bootTime',
      'lastCheck',
      'nextRun',
      'startedAt',
      'finishedAt',
      'checkedAt',
      'generatedAt',
    ];
    for (const key of keys) {
      for (const value of valuesOf(all, key)) {
        if (value !== null) expect(Number.isInteger(value), `${key}=${String(value)}`).toBe(true);
      }
    }
  });

  it('includes host on every response except health and error', () => {
    for (const [file, body] of Object.entries(fixtures)) {
      if (file === 'health.json' || file === 'error.json') continue;
      expect(body, file).toHaveProperty('host', 'debian-box');
    }
  });

  it('counts each overview target exactly once', () => {
    const o = fixtures['overview.json'];
    const library = o.library === null ? 0 : 1;
    const targets = 1 + o.services.length + o.timers.length + o.checks.length + library;
    const counted = Object.values(o.counts).reduce((sum, n) => sum + n, 0);
    expect(counted).toBe(targets);
    expect(Object.keys(o.counts).sort()).toEqual([...HEALTH_STATES].sort());
  });

  it('lists events and runs newest first', () => {
    const lists: { ts: number }[][] = [
      fixtures['events.json'].events,
      fixtures['overview.json'].recentEvents,
      fixtures['timer-detail.json'].runs.map((r) => ({ ts: r.startedAt })),
    ];
    for (const list of lists) {
      const ts = list.map((e) => e.ts);
      expect(ts).toEqual([...ts].sort((a, b) => b - a));
    }
  });

  it('returns metric points as [ts, avg, min, max] on the step grid', () => {
    const q = fixtures['metrics-query.json'];
    for (const series of q.series) {
      for (const [ts, ...values] of series.points) {
        expect(values).toHaveLength(3);
        expect((ts - q.from) % q.step).toBe(0);
        expect(ts).toBeGreaterThanOrEqual(q.from);
        expect(ts).toBeLessThan(q.to);
      }
    }
  });

  it('reports health ok with a version', () => {
    expect(fixtures['health.json'].status).toBe('ok');
    expect(fixtures['health.json'].version).toMatch(/^\d+\.\d+\.\d+/);
  });
});

describe('library examples', () => {
  const configured = fixtures['library.json'];
  const unconfigured = fixtures['library-unconfigured.json'];

  it('carries the same summary as the overview', () => {
    expect(configured.configured).toBe(true);
    expect(configured.library).toEqual(fixtures['overview.json'].library);
  });

  it('is all null without [library]', () => {
    expect(unconfigured).toEqual({
      host: 'debian-box',
      configured: false,
      library: null,
      report: null,
    });
  });

  it('orders issues by severity and keeps the counts consistent', () => {
    const report = configured.report!;
    const rank = { error: 0, warning: 1, info: 2 };
    for (const album of report.albums) {
      const ranks = album.issues.map((i) => rank[i.severity]);
      expect(ranks).toEqual([...ranks].sort((a, b) => a - b));
      for (const issue of album.issues) {
        expect(issue.scope).not.toBe('library');
        expect(issue.pathsTotal).toBeGreaterThanOrEqual(issue.paths.length);
      }
    }
    for (const issue of report.libraryIssues) expect(issue.scope).toBe('library');
    const kept = report.albums.flatMap((a) => a.issues).length + report.libraryIssues.length;
    expect(kept).toBe(report.issuesTotal);
    expect(report.errors).toHaveLength(report.errorsTotal);
  });
});

// Proves the compile-time checker in contract.ts actually rejects drift. These lines are verified
// by `pnpm check` (an unused @ts-expect-error is itself an error), not by vitest.
describe('Conforms<J, T>', () => {
  it('is enforced by the type checker', () => {
    const event = {
      id: 1,
      ts: 1,
      targetKind: 'service',
      target: 'x.service',
      from: 'healthy',
      to: 'warning',
      message: null,
    };
    event satisfies Conforms<typeof event, HealthEvent>;

    const { message: _omitted, ...missingKey } = event;
    // @ts-expect-error a contract field is missing
    missingKey satisfies Conforms<typeof missingKey, HealthEvent>;

    const extraKey = { ...event, extra: true };
    // @ts-expect-error a key the contract does not declare
    extraKey satisfies Conforms<typeof extraKey, HealthEvent>;

    const wrongType = { ...event, id: '1' };
    // @ts-expect-error a field has the wrong primitive type
    wrongType satisfies Conforms<typeof wrongType, HealthEvent>;

    const timer = fixtures['timers.json'].timers[0] as TimerSummary;
    const nestedExtra = { ...timer, lastRun: { ...timer.lastRun, pid: 1 } };
    // @ts-expect-error an undeclared key inside a nested object
    nestedExtra satisfies Conforms<typeof nestedExtra, TimerSummary>;

    expect(true).toBe(true);
  });
});

// Null cases the docs examples do not contain yet; `satisfies` makes `pnpm check` verify them.
describe('nullable contract fields', () => {
  it('accepts an event with no previous state', () => {
    const firstEvent = {
      id: 1,
      ts: 1794200000,
      targetKind: 'timer',
      target: 'backup.timer',
      from: null,
      to: 'unknown',
      message: 'never run',
    };
    firstEvent satisfies Conforms<typeof firstEvent, HealthEvent>;
    const typed: HealthEvent = { ...firstEvent, targetKind: 'timer', to: 'unknown' };
    expect(typed.from).toBeNull();
  });

  it('accepts a running timer with no next run', () => {
    const runningTimer = {
      unit: 'backup.timer',
      name: 'Backup',
      service: 'backup.service',
      health: 'healthy',
      message: null,
      active: true,
      lastRun: {
        startedAt: 1794199900,
        finishedAt: null,
        duration: null,
        result: null,
        exitStatus: null,
      },
      nextRun: null,
      running: true,
    };
    runningTimer satisfies Conforms<typeof runningTimer, TimerSummary>;
    const typed: TimerSummary = { ...runningTimer, health: 'healthy' };
    expect(typed.nextRun).toBeNull();
    expect(typed.lastRun).toMatchObject({ finishedAt: null, duration: null, result: null });
  });
  it('accepts a service that was never active', () => {
    const neverActive = {
      unit: 'missing.service',
      name: 'missing.service',
      description: 'missing.service',
      health: 'critical',
      message: 'unit not found',
      loadState: 'not-found',
      activeState: 'inactive',
      subState: 'dead',
      since: null,
      restarts: 0,
      memory: null,
    };
    neverActive satisfies Conforms<typeof neverActive, ServiceSummary>;
    const typed: ServiceSummary = { ...neverActive, health: 'critical' };
    expect(typed.since).toBeNull();
  });

  it('accepts a check that has not run yet', () => {
    const notRun = {
      name: 'Game port',
      kind: 'tcp',
      target: '127.0.0.1:25565',
      health: 'unknown',
      message: 'not yet run',
      lastCheck: null,
      latencyMs: null,
      consecutiveFailures: 0,
    };
    notRun satisfies Conforms<typeof notRun, CheckSummary>;
    const typed: CheckSummary = { ...notRun, kind: 'tcp', health: 'unknown' };
    expect(typed).toMatchObject({ lastCheck: null, latencyMs: null });
  });
});
