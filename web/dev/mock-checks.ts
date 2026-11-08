// Generated checks for the dev mock, one per state the UI has to show: healthy, slow (warning),
// just started failing (warning), failing for a while (critical) and not run yet (unknown). Like
// the other mock modules, everything is a pure function of the server time `now`. Targets are
// shown as the backend sanitizes them (no userinfo, no query string).
//
// Check events are logged without hold-down (the consecutive-failure rule already debounces): a
// failing check logs healthy → warning on its first failure and warning → critical on its third.
// Failed probes have no latency, so their series has a gap from the first failure.

import type { CheckSummary, Health, HealthEvent } from '../src/lib/api/types';

interface CheckSpec {
  summary: Omit<CheckSummary, 'lastCheck'>;
  interval: number;
  /** Whether it has run at all. */
  ran: boolean;
  /** A healthy-but-slow check: seconds before `now` that it turned warning, for its event. */
  since?: number;
  /** A failing check: why each probe failed; messages append "(n consecutive failures)". */
  failure?: string;
}

/** Seconds before `now` that the current run of failures began (one interval per failure). */
function failingFor(spec: CheckSpec): number {
  return spec.summary.consecutiveFailures * spec.interval;
}

function failureMessage(reason: string, n: number): string {
  return `${reason} (${n} consecutive ${n === 1 ? 'failure' : 'failures'})`;
}

const SPECS: CheckSpec[] = [
  {
    summary: {
      name: 'Music HTTP',
      kind: 'http',
      target: 'http://127.0.0.1:4533/ping',
      health: 'healthy',
      message: null,
      latencyMs: 3.1,
      consecutiveFailures: 0,
    },
    interval: 60,
    ran: true,
  },
  {
    summary: {
      name: 'Photos API',
      kind: 'http',
      target: 'http://127.0.0.1:2342/api/v1/status',
      health: 'warning',
      message: 'slow response: 1.42 s (warning ≥ 1 s)',
      latencyMs: 1420,
      consecutiveFailures: 0,
    },
    interval: 60,
    ran: true,
    since: 25 * 60,
  },
  {
    summary: {
      name: 'Game port',
      kind: 'tcp',
      target: '127.0.0.1:25565',
      health: 'warning',
      message: failureMessage('connection refused', 1),
      latencyMs: null,
      consecutiveFailures: 1,
    },
    interval: 30,
    ran: true,
    failure: 'connection refused',
  },
  {
    summary: {
      name: 'Grafana',
      kind: 'http',
      target: 'http://127.0.0.1:3000/api/health',
      health: 'critical',
      message: failureMessage('HTTP 502 Bad Gateway', 4),
      latencyMs: null,
      consecutiveFailures: 4,
    },
    interval: 60,
    ran: true,
    failure: 'HTTP 502 Bad Gateway',
  },
  {
    summary: {
      name: 'Home Assistant',
      kind: 'http',
      target: 'http://127.0.0.1:8123/',
      health: 'unknown',
      message: 'not yet run',
      latencyMs: null,
      consecutiveFailures: 0,
    },
    interval: 300,
    ran: false,
  },
];

/** Names of checks with a latency series (every check that has answered at least once). */
export const LATENCY_CHECKS = SPECS.filter((s) => s.ran).map((s) => s.summary.name);

export function checks(now: number): CheckSummary[] {
  return SPECS.map((spec) => ({
    ...spec.summary,
    // The last probe sits on the check's own schedule.
    lastCheck: spec.ran ? Math.floor(now / spec.interval) * spec.interval : null,
  }));
}

export function check(name: string, now: number): CheckSummary | null {
  return checks(now).find((c) => c.name === name) ?? null;
}

/** Transitions into each check's current state, as `events` drafts without ids. */
export function checkEvents(now: number): Omit<HealthEvent, 'id'>[] {
  return SPECS.flatMap((spec) => {
    const { name, health, message, consecutiveFailures: n } = spec.summary;
    // Event times sit on the probe schedule, like `lastCheck`.
    const last = Math.floor(now / spec.interval) * spec.interval;
    const event = (ts: number, from: Health, to: Health, text: string | null) => ({
      ts,
      targetKind: 'check' as const,
      target: name,
      from,
      to,
      message: text,
    });
    if (spec.failure !== undefined && n > 0) {
      // Failure k happened (n - k) intervals before the latest probe (ARCHITECTURE §8 thresholds).
      const at = (k: number) => last - (n - k) * spec.interval;
      const drafts = [event(at(1), 'healthy', 'warning', failureMessage(spec.failure, 1))];
      if (n >= 3) drafts.push(event(at(3), 'warning', 'critical', failureMessage(spec.failure, 3)));
      return drafts;
    }
    if (spec.since !== undefined) return [event(last - spec.since, 'healthy', health, message)];
    return [];
  });
}

/** True while `name` is failing at `t`: failed probes record no latency, so the series has a gap. */
export function latencyMissing(name: string, t: number, now: number): boolean {
  const spec = SPECS.find((s) => s.summary.name === name);
  if (!spec || spec.summary.consecutiveFailures === 0) return false;
  const last = Math.floor(now / spec.interval) * spec.interval;
  // The last successful probe was `failures` intervals before the latest one.
  return t > last - failingFor(spec);
}

/** Latency in ms at `t` for a check's series; `jitter` is in [0, 1). */
export function latencySample(name: string, jitter: number, daily: number): number {
  switch (name) {
    case 'Photos API':
      // Has been creeping up with the evening indexing load.
      return 600 + 500 * (daily + 1) + 200 * jitter;
    case 'Grafana':
      return 18 + 6 * jitter ** 3;
    case 'Game port':
      return 0.4 + 0.3 * jitter;
    default:
      return 2.5 + 2 * jitter ** 4 + (jitter > 0.995 ? 120 : 0);
  }
}
