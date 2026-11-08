import { describe, expect, it } from 'vitest';
import type {
  EventsResponse,
  Overview,
  ServiceDetail,
  TimerDetail,
  TimersResponse,
} from '../src/lib/api/types';
import { respond } from './mock-api';

const NOW = 1_794_200_000;

function get<T>(path: string, query = '', now = NOW): { status: number; body: T } {
  const { status, body } = respond(path, new URLSearchParams(query), false, now);
  return { status, body: body as T };
}

describe('mock timers', () => {
  it('covers failed, never-run, running, healthy and stopped timers', () => {
    const { timers } = get<TimersResponse>('/timers').body;
    const by = (unit: string) => timers.find((t) => t.unit === unit);
    expect(by('music-sync.timer')).toMatchObject({ health: 'critical', running: false });
    expect(by('backup.timer')).toMatchObject({ health: 'unknown', lastRun: null });
    expect(by('photos-index.timer')).toMatchObject({ health: 'healthy', running: true });
    expect(by('photos-index.timer')?.lastRun?.result).toBeNull();
    expect(by('certbot.timer')).toMatchObject({ health: 'healthy', active: true });
    expect(by('apt-daily.timer')).toMatchObject({
      health: 'warning',
      active: false,
      nextRun: null,
    });
    for (const t of timers) if (t.nextRun !== null) expect(t.nextRun).toBeGreaterThan(NOW);
  });

  it('keeps run start times stable between polls', () => {
    const a = get<TimerDetail>('/timers/certbot.timer').body.runs;
    const b = get<TimerDetail>('/timers/certbot.timer', '', NOW + 600).body.runs;
    expect(b[0]?.startedAt).toBe(a[0]?.startedAt);
  });

  it('serves run history newest first, honouring limit', () => {
    const { runs, timer } = get<TimerDetail>('/timers/music-sync.timer', 'limit=5').body;
    expect(runs).toHaveLength(5);
    expect(runs[0]?.startedAt).toBe(timer.lastRun?.startedAt);
    expect(runs[0]?.result).toBe('exit-code');
    const starts = runs.map((r) => r.startedAt);
    expect([...starts].sort((x, y) => y - x)).toEqual(starts);
  });

  it.each([
    ['minecraft.service', 1_794_200_000],
    ['minecraft.service', 1_794_200_000 + 13 * 3600],
    ['navidrome.service', 1_794_200_000],
  ])('ends the %s restart series at the summary count (now %d)', (unit, now) => {
    const service = get<ServiceDetail>(`/services/${unit}`, '', now).body.service;
    const query = get<{ series: { points: [number, number | null][] }[] }>(
      '/metrics/query',
      `key=service.restarts:${unit}&from=${now - 86_400}&to=${now}`,
      now,
    ).body;
    const values = query.series[0]!.points.map((p) => p[1]).filter((v) => v !== null);
    expect(values.at(-1)).toBe(service.restarts);
    // A failed unit stops restarting: the count is flat after the failure.
    if (service.since !== null && service.activeState === 'failed') {
      const after = query.series[0]!.points.filter((p) => p[0] >= service.since! && p[1] !== null);
      expect(new Set(after.map((p) => p[1]))).toEqual(new Set([service.restarts]));
    }
  });

  it('answers 404 for unknown units', () => {
    expect(get('/timers/nope.timer').status).toBe(404);
    expect(get('/services/nope.service').status).toBe(404);
  });

  it('serves service detail with its events', () => {
    const { status, body } = get<ServiceDetail>('/services/minecraft.service');
    expect(status).toBe(200);
    expect(body.service.health).toBe('critical');
    expect(body.events[0]).toMatchObject({ target: 'minecraft.service', to: 'critical' });
  });

  it('keeps overview timers and counts consistent with /timers', () => {
    const overview = get<Overview>('/overview').body;
    expect(overview.timers).toEqual(get<TimersResponse>('/timers').body.timers);
    const total = Object.values(overview.counts).reduce((a, b) => a + b, 0);
    expect(total).toBe(
      1 + overview.services.length + overview.timers.length + overview.checks.length,
    );
    expect(overview.recentEvents).toHaveLength(10);
  });
});

describe('mock /events', () => {
  it('pages back with an exclusive until in whole seconds, ending with an empty page', () => {
    const all = get<EventsResponse>('/events', 'limit=1000').body.events;
    expect(all.length).toBeGreaterThan(100);
    const seen: number[] = [];
    let until: number | undefined;
    for (let i = 0; i < 100; i += 1) {
      const query = `limit=50${until === undefined ? '' : `&until=${until}`}`;
      const page = get<EventsResponse>('/events', query).body.events;
      if (page.length === 0) break;
      const oldest = page.at(-1)!.ts;
      // Whole seconds: nothing of the oldest second is left for the next page.
      const ids = new Set([...seen, ...page.map((e) => e.id)]);
      expect(all.filter((e) => e.ts === oldest).every((e) => ids.has(e.id))).toBe(true);
      expect(page.some((e) => until !== undefined && e.ts >= until)).toBe(false);
      seen.push(...page.map((e) => e.id));
      until = oldest;
    }
    expect(seen).toEqual(all.map((e) => e.id));
  });

  it('filters by kind, combinable with target', () => {
    const timers = get<EventsResponse>('/events', 'kind=timer&limit=1000').body.events;
    expect(timers.length).toBeGreaterThan(0);
    expect(timers.every((e) => e.targetKind === 'timer')).toBe(true);
    expect(
      get<EventsResponse>('/events', 'kind=service&target=music-sync.timer').body.events,
    ).toEqual([]);
  });

  it('answers an unknown metric key with unknown_series', () => {
    const { status, body } = get<{ error: { code: string } }>(
      '/metrics/query',
      'key=service.memory:minecraft.service',
    );
    expect(status).toBe(400);
    expect(body.error.code).toBe('unknown_series');
  });

  it('filters by target and since', () => {
    const { events } = get<EventsResponse>(
      '/events',
      `target=music-sync.timer&since=${NOW - 10 * 86_400}`,
    ).body;
    expect(events.length).toBeGreaterThan(0);
    expect(events.every((e) => e.target === 'music-sync.timer' && e.ts >= NOW - 10 * 86_400)).toBe(
      true,
    );
  });

  it('records a failed timer run and the recovery after it', () => {
    const { events } = get<EventsResponse>('/events', 'target=music-sync.timer&limit=1000').body;
    expect(events[0]).toMatchObject({ from: 'healthy', to: 'critical' });
    expect(events.some((e) => e.from === 'critical' && e.to === 'healthy')).toBe(true);
  });

  it('gives an event the same id in every response', () => {
    const a = get<EventsResponse>('/events').body.events.at(-1)!;
    const b = get<EventsResponse>('/events', `until=${a.ts + 1}&limit=5`, NOW + 120).body.events;
    expect(b.find((e) => e.id === a.id)).toEqual(a);
  });
});
