// Generated timers, timer runs and events for the dev mock. Like mock-metrics.ts, everything is a
// pure function of the server time `now`: run times sit on fixed schedules, so a run keeps its
// start time (and an event its id) from one poll to the next, and paging through `/events` with
// `until` works across requests.
//
// The timers cover every state the UI has to show: a failed last run, never run, running right
// now, healthy with varied durations, and a stopped timer. Events are derived from the same runs
// (a failure and the recovery after it), plus service, system and check transitions.

import type {
  HealthEvent,
  ServiceSummary,
  TargetKind,
  TimerRun,
  TimerSummary,
} from '../src/lib/api/types';

const HOUR = 3600;
const DAY = 24 * HOUR;
/** How far back runs and events are generated. */
const HISTORY = 30 * DAY;

interface TimerSpec {
  unit: string;
  name: string;
  period: number;
  /** Seconds after each period boundary that the run starts. */
  offset: number;
  /** Typical run length in seconds; actual runs vary ±40%. */
  duration: number;
  /** Runs (counted back from the latest, 0 = latest) that fail, with their systemd result. */
  failures: Record<number, { result: string; exitStatus: number }>;
  /** The latest run is still going. */
  running?: boolean;
  never?: boolean;
  /** Stopped: no next run, and runs ended `stoppedFor` seconds ago. */
  stoppedFor?: number;
}

const TIMERS: TimerSpec[] = [
  {
    unit: 'music-sync.timer',
    name: 'Music sync',
    period: DAY,
    offset: 3 * HOUR,
    duration: 40,
    failures: {
      0: { result: 'exit-code', exitStatus: 1 },
      6: { result: 'timeout', exitStatus: 0 },
    },
  },
  {
    unit: 'backup.timer',
    name: 'backup.timer',
    period: DAY,
    offset: 2 * HOUR,
    duration: 0,
    failures: {},
    never: true,
  },
  {
    unit: 'photos-index.timer',
    name: 'Photo index',
    period: 15 * 60,
    offset: 0,
    duration: 9 * 60,
    failures: { 11: { result: 'exit-code', exitStatus: 2 } },
    running: true,
  },
  {
    unit: 'certbot.timer',
    name: 'Certificate renewal',
    period: 12 * HOUR,
    offset: 41 * 60,
    duration: 6,
    failures: {},
  },
  {
    unit: 'apt-daily.timer',
    name: 'apt-daily.timer',
    period: DAY,
    offset: 6 * HOUR,
    duration: 95,
    failures: {},
    stoppedFor: 3 * DAY,
  },
];

/** Deterministic value in [0, 1) for a string and integer. */
function noise(key: string, n: number): number {
  let h = 2166136261 ^ n;
  for (let i = 0; i < key.length; i += 1) h = Math.imul(h ^ key.charCodeAt(i), 16777619);
  h = Math.imul(h ^ (h >>> 15), 2246822507);
  h = Math.imul(h ^ (h >>> 13), 3266489909);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
}

/** Start of the latest run at or before `now`, on the timer's schedule. */
function latestStart(spec: TimerSpec, now: number): number {
  const end = now - (spec.stoppedFor ?? 0);
  return Math.floor((end - spec.offset) / spec.period) * spec.period + spec.offset;
}

/** Newest first, at most `limit`. */
function runsOf(spec: TimerSpec, now: number, limit: number): TimerRun[] {
  if (spec.never) return [];
  const latest = latestStart(spec, now);
  const runs: TimerRun[] = [];
  for (let k = 0; runs.length < limit && k * spec.period < HISTORY; k += 1) {
    const startedAt = latest - k * spec.period;
    const slot = Math.round(startedAt / spec.period);
    const duration = Math.max(1, Math.round(spec.duration * (0.6 + 0.8 * noise(spec.unit, slot))));
    // Older runs come from journal backfill, recent ones were observed live.
    const source = k < 8 ? 'observed' : 'journal';
    if (k === 0 && spec.running) {
      runs.push({
        startedAt,
        finishedAt: null,
        duration: null,
        result: null,
        exitStatus: null,
        source,
      });
      continue;
    }
    const failure = spec.failures[k];
    // A timed-out run is killed at the limit, so it ran long.
    const took = failure?.result === 'timeout' ? duration * 4 : duration;
    runs.push({
      startedAt,
      finishedAt: startedAt + took,
      duration: took,
      result: failure?.result ?? 'success',
      exitStatus: failure ? failure.exitStatus : 0,
      source,
    });
  }
  return runs;
}

function summaryOf(spec: TimerSpec, now: number): TimerSummary {
  const [last] = runsOf(spec, now, 1);
  const lastRun = last
    ? {
        startedAt: last.startedAt,
        finishedAt: last.finishedAt,
        duration: last.duration,
        result: last.result,
        exitStatus: last.exitStatus,
      }
    : null;
  const stopped = spec.stoppedFor !== undefined;
  const nextRun = stopped ? null : latestStart(spec, now) + spec.period;
  const failed = last !== undefined && last.result !== null && last.result !== 'success';

  let health: TimerSummary['health'] = 'healthy';
  let message: string | null = null;
  if (failed) {
    health = 'critical';
    const status = last.exitStatus ? ` (status ${last.exitStatus})` : '';
    message = `last run failed: ${last.result}${status}`;
  } else if (stopped) {
    health = 'warning';
    message = 'timer is not active';
  } else if (!last) {
    health = 'unknown';
    message = 'never run';
  }
  return {
    unit: spec.unit,
    name: spec.name,
    service: spec.unit.replace(/\.timer$/, '.service'),
    health,
    message,
    active: !stopped,
    lastRun,
    nextRun,
    running: last !== undefined && last.result === null,
  };
}

export function timers(now: number): TimerSummary[] {
  return TIMERS.map((spec) => summaryOf(spec, now));
}

/** Null for a timer the mock does not know. */
export function timerDetail(unit: string, now: number, limit: number) {
  const spec = TIMERS.find((t) => t.unit === unit);
  if (!spec) return null;
  return { timer: summaryOf(spec, now), runs: runsOf(spec, now, limit) };
}

// ---- Events ---------------------------------------------------------------------------------

type Draft = Omit<HealthEvent, 'id'>;

/**
 * Ids grow with time and never collide: the timestamp times 16 plus a per-source index, so the
 * same event has the same id in every response.
 */
function withIds(drafts: Draft[], source: number): HealthEvent[] {
  return drafts.map((d) => ({ id: d.ts * 16 + source, ...d }));
}

function timerEvents(spec: TimerSpec, now: number): Draft[] {
  const runs = runsOf(spec, now, Infinity).reverse(); // oldest first
  const drafts: Draft[] = [];
  let state: HealthEvent['to'] | null = null;
  const move = (ts: number, to: HealthEvent['to'], message: string | null) => {
    if (state === to) return;
    drafts.push({ ts, targetKind: 'timer', target: spec.unit, from: state, to, message });
    state = to;
  };
  if (runs.length === 0) move(now - HISTORY, 'unknown', 'never run');
  for (const run of runs) {
    if (run.finishedAt === null || run.result === null) continue;
    if (run.result === 'success') move(run.finishedAt, 'healthy', null);
    else {
      const status = run.exitStatus ? ` (status ${run.exitStatus})` : '';
      move(run.finishedAt, 'critical', `last run failed: ${run.result}${status}`);
    }
  }
  if (spec.stoppedFor !== undefined) {
    move(now - spec.stoppedFor + spec.period + 10 * 60, 'warning', 'timer is not active');
  }
  return drafts;
}

/** A target that goes bad for `length` seconds once per `period`, at a phase set by its name. */
function flapping(
  kind: TargetKind,
  target: string,
  period: number,
  length: number,
  to: HealthEvent['to'],
  message: string,
  now: number,
): Draft[] {
  const phase = Math.floor(noise(target, 0) * period);
  const drafts: Draft[] = [];
  const first = Math.ceil((now - HISTORY - phase) / period);
  for (let n = first; ; n += 1) {
    const start = n * period + phase;
    if (start > now) break;
    drafts.push({ ts: start, targetKind: kind, target, from: 'healthy', to, message });
    if (start + length <= now) {
      drafts.push({
        ts: start + length,
        targetKind: kind,
        target,
        from: to,
        to: 'healthy',
        message: null,
      });
    }
  }
  return drafts;
}

function allEvents(now: number, services: readonly ServiceSummary[]): HealthEvent[] {
  const sources: Draft[][] = TIMERS.map((spec) => timerEvents(spec, now));
  for (const service of services) {
    const drafts: Draft[] = [
      {
        ts: now - HISTORY,
        targetKind: 'service',
        target: service.unit,
        from: null,
        to: 'healthy',
        message: null,
      },
    ];
    if (service.health !== 'healthy' && service.since !== null) {
      drafts.push({
        ts: service.since,
        targetKind: 'service',
        target: service.unit,
        from: 'healthy',
        to: service.health,
        message: service.message,
      });
    }
    sources.push(drafts);
  }
  sources.push(
    flapping('system', 'cpu', 19 * HOUR, 7 * 60, 'warning', 'cpu at 91% (warning ≥ 85%)', now),
    flapping('system', 'disk:/', 9 * DAY, 4 * DAY, 'warning', 'disk / at 84% (warning ≥ 80%)', now),
    flapping('check', 'Music HTTP', 4 * DAY, 3 * 60, 'warning', '1 consecutive failure', now),
  );
  return sources
    .flatMap((drafts, source) => withIds(drafts, source))
    .filter((e) => e.ts <= now)
    .sort((a, b) => b.ts - a.ts || b.id - a.id);
}

export interface EventQuery {
  since?: number;
  /** Exclusive. */
  until?: number;
  target?: string;
  kind?: TargetKind;
  limit?: number;
}

/**
 * Newest first, as the backend pages them: whole seconds only, so a page runs past `limit` to
 * finish the second it stopped in. `services` are the configured services whose transitions
 * are included.
 */
export function events(
  now: number,
  services: readonly ServiceSummary[],
  query: EventQuery = {},
): HealthEvent[] {
  const limit = Math.min(1000, Math.max(1, query.limit ?? 100));
  const matching = allEvents(now, services).filter(
    (e) =>
      (query.target === undefined || e.target === query.target) &&
      (query.kind === undefined || e.targetKind === query.kind) &&
      (query.since === undefined || e.ts >= query.since) &&
      (query.until === undefined || e.ts < query.until),
  );
  const last = matching[limit - 1];
  return last === undefined ? matching : matching.filter((e, i) => i < limit || e.ts === last.ts);
}
