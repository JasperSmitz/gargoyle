import { describe, expect, it } from 'vitest';
import { HEALTH_STATES, type Health, type Overview } from './api/types';
import { fixtures } from './api/fixtures';
import {
  HEALTH_BY_SEVERITY,
  HEALTH_PRESENTATION,
  needsAttention,
  severity,
  sortBySeverity,
  worst,
} from './health';
import { collectProblems } from './problems';

describe('health presentation', () => {
  it('gives every state its own label and icon, so color is never the only signal', () => {
    const labels = HEALTH_STATES.map((h) => HEALTH_PRESENTATION[h].label);
    const icons = HEALTH_STATES.map((h) => HEALTH_PRESENTATION[h].icon);
    expect(new Set(labels).size).toBe(HEALTH_STATES.length);
    expect(new Set(icons).size).toBe(HEALTH_STATES.length);
  });

  it('orders critical > warning > unknown > healthy (ARCHITECTURE §8)', () => {
    expect(HEALTH_BY_SEVERITY).toEqual(['critical', 'warning', 'unknown', 'healthy']);
    expect(severity('critical')).toBeGreaterThan(severity('warning'));
    expect(severity('warning')).toBeGreaterThan(severity('unknown'));
    expect(severity('unknown')).toBeGreaterThan(severity('healthy'));
  });

  it.each<[Health[], Health]>([
    [[], 'healthy'],
    [['healthy', 'healthy'], 'healthy'],
    [['healthy', 'unknown'], 'unknown'],
    [['unknown', 'warning', 'healthy'], 'warning'],
    [['warning', 'critical', 'unknown'], 'critical'],
  ])('worst(%j) = %s', (states, expected) => {
    expect(worst(states)).toBe(expected);
  });

  it('treats everything but healthy as needing attention', () => {
    expect(HEALTH_STATES.filter(needsAttention)).toEqual(['warning', 'critical', 'unknown']);
  });

  it('sorts worst first and keeps configured order within a state', () => {
    const items = [
      { id: 'a', health: 'healthy' as const },
      { id: 'b', health: 'unknown' as const },
      { id: 'c', health: 'critical' as const },
      { id: 'd', health: 'healthy' as const },
      { id: 'e', health: 'critical' as const },
    ];
    expect(sortBySeverity(items).map((i) => i.id)).toEqual(['c', 'e', 'b', 'a', 'd']);
    expect(items[0]?.id).toBe('a'); // input untouched
  });
});

describe('collectProblems', () => {
  const overview = fixtures['overview.json'];

  it('lists every non-healthy target of the example overview, worst first', () => {
    expect(collectProblems(overview).map((p) => [p.health, p.name, p.message])).toEqual([
      ['critical', 'Minecraft', 'unit failed (Result=exit-code)'],
      ['critical', 'Music sync', 'last run failed: exit-code (status 1)'],
      ['warning', 'Disk /', '84% used'],
      ['unknown', 'backup.timer', 'never run'],
    ]);
  });

  it('is empty when everything is healthy', () => {
    const allHealthy = structuredClone(overview);
    allHealthy.system.disks.forEach((d) => (d.health = 'healthy'));
    allHealthy.system.health = 'healthy';
    allHealthy.services.forEach((s) => (s.health = 'healthy'));
    allHealthy.timers.forEach((t) => (t.health = 'healthy'));
    expect(collectProblems(allHealthy)).toEqual([]);
  });

  it('reports system health that no individual metric explains', () => {
    const stale: Overview = structuredClone(overview);
    stale.system.disks.forEach((d) => (d.health = 'healthy'));
    stale.system.health = 'unknown';
    expect(collectProblems(stale)).toContainEqual(
      expect.objectContaining({ kind: 'system', name: 'System', health: 'unknown' }),
    );
  });

  it('collapses system metrics into one line before the first sample (ts = 0)', () => {
    const fresh: Overview = structuredClone(overview);
    fresh.system.ts = 0;
    fresh.system.health = 'unknown';
    fresh.system.cpu.health = 'unknown';
    fresh.system.memory.health = 'unknown';
    const system = collectProblems(fresh).filter((p) => p.kind === 'system');
    expect(system).toEqual([
      expect.objectContaining({ name: 'System', message: 'waiting for the first sample' }),
    ]);
  });

  it('falls back to the state label when a non-healthy target has no message', () => {
    const quiet: Overview = structuredClone(overview);
    const service = quiet.services[1];
    if (!service) throw new Error('fixture has two services');
    service.message = null;
    expect(collectProblems(quiet)).toContainEqual(
      expect.objectContaining({ name: 'Minecraft', message: 'Critical' }),
    );
  });
});
