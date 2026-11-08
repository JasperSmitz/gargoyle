import { describe, expect, it } from 'vitest';
import { respond } from './mock-api';

const NOW = 1_794_200_000;

function get(path: string, query: string) {
  return respond(path, new URLSearchParams(query), false, NOW);
}

describe('mock /metrics/query', () => {
  it.each([
    ['1h', 3600, 'raw'],
    ['6h', 6 * 3600, 'raw'],
    ['24h', 86_400, '5m'],
    ['7d', 7 * 86_400, '5m'],
    ['30d', 30 * 86_400, '5m'],
    ['1y', 365 * 86_400, '5m'],
  ])('serves %s within the requested points', (_name, span, resolution) => {
    const { status, body } = get(
      '/metrics/query',
      `key=cpu.usage&key=net.rx:*&from=${NOW - span}&to=${NOW}&points=300`,
    );
    expect(status).toBe(200);
    const result = body as {
      resolution: string;
      step: number;
      series: { key: string; points: unknown[][] }[];
    };
    expect(result.resolution).toBe(resolution);
    expect(result.series.map((s) => s.key)).toEqual(['cpu.usage', 'net.rx:enp1s0']);
    for (const s of result.series) {
      expect(s.points.length).toBeGreaterThanOrEqual(150);
      expect(s.points.length).toBeLessThanOrEqual(300);
    }
  });

  it('is deterministic and has gaps somewhere in a week', () => {
    const query = `key=mem.used&from=${NOW - 7 * 86_400}&to=${NOW}`;
    expect(get('/metrics/query', query)).toEqual(get('/metrics/query', query));
    const { body } = get('/metrics/query', query);
    const points = (body as { series: { points: (number | null)[][] }[] }).series[0]?.points ?? [];
    expect(points.some((p) => p[1] === null)).toBe(true);
    expect(points.some((p) => p[1] !== null)).toBe(true);
  });

  it('rejects unknown keys with 400', () => {
    expect(get('/metrics/query', 'key=nope').status).toBe(400);
    expect(get('/metrics/query', '').status).toBe(400);
  });

  it('lists the generated series, temperatures included', () => {
    const { body } = get('/metrics', '');
    const keys = (body as { series: { key: string }[] }).series.map((s) => s.key);
    expect(keys).toContain('temp:coretemp/Package id 0');
    expect(keys).toContain('net.tx:enp1s0');
  });
});
