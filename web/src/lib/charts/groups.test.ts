import { describe, expect, it } from 'vitest';
import { fixtures } from '../api/fixtures';
import type { SeriesInfo, SeriesUnit } from '../api/types';
import { buildChartGroups, buildQuery } from './groups';
import { rangeById } from './range';

function info(key: string, unit: SeriesUnit): SeriesInfo {
  const [metric = key, label = null] = key.includes(':')
    ? [key.slice(0, key.indexOf(':')), key.slice(key.indexOf(':') + 1)]
    : [key, null];
  return { key, metric, label, unit };
}

const FULL = [
  info('cpu.usage', 'percent'),
  info('cpu.iowait', 'percent'),
  info('load.1', 'load'),
  info('load.5', 'load'),
  info('load.15', 'load'),
  info('mem.used', 'percent'),
  info('swap.used', 'percent'),
  info('disk.used:/srv', 'percent'),
  info('disk.used:/', 'percent'),
  info('net.rx:wlan0', 'bytes_per_s'),
  info('net.tx:wlan0', 'bytes_per_s'),
  info('net.rx:enp1s0', 'bytes_per_s'),
  info('net.tx:enp1s0', 'bytes_per_s'),
  info('temp:acpitz', 'celsius'),
  info('check.latency:Music HTTP', 'ms'),
];

describe('buildChartGroups', () => {
  it('builds CPU, load, memory, disk, network and temperature charts', () => {
    const groups = buildChartGroups(FULL);
    expect(groups.map((g) => g.id)).toEqual(['cpu', 'load', 'memory', 'disk', 'net', 'temps']);
    expect(groups.flatMap((g) => g.charts.map((c) => c.title))).toEqual([
      'CPU',
      'Load average',
      'Memory and swap',
      'Disk usage',
      'Network enp1s0',
      'Network wlan0',
      'Temperatures',
    ]);
  });

  it('queries labelled metrics with globs', () => {
    const byId = new Map(buildChartGroups(FULL).map((g) => [g.id, g]));
    expect(byId.get('cpu')?.queryKeys).toEqual(['cpu.usage', 'cpu.iowait']);
    expect(byId.get('disk')?.queryKeys).toEqual(['disk.used:*']);
    expect(byId.get('net')?.queryKeys).toEqual(['net.rx:*', 'net.tx:*']);
    expect(byId.get('temps')?.queryKeys).toEqual(['temp:*']);
  });

  it('hides the temperature chart entirely when there are no temperature series', () => {
    const groups = buildChartGroups(FULL.filter((s) => s.metric !== 'temp'));
    expect(groups.map((g) => g.id)).not.toContain('temps');
  });

  it('builds charts from what the backend lists (the contract example)', () => {
    const groups = buildChartGroups(fixtures['metrics.json'].series);
    const cpu = groups.find((g) => g.id === 'cpu');
    expect(cpu?.charts[0]?.series.map((s) => s.key)).toEqual(['cpu.usage']);
    expect(groups.find((g) => g.id === 'net')?.charts[0]?.series.map((s) => s.label)).toEqual([
      'Received',
    ]);
  });

  it('assigns colors per entity, not by rank', () => {
    // Without cpu.usage, iowait keeps its own slot instead of moving up into slot 0.
    const groups = buildChartGroups(FULL.filter((s) => s.key !== 'cpu.usage'));
    expect(groups[0]?.charts[0]?.series).toEqual([
      { key: 'cpu.iowait', label: 'I/O wait', slot: 1 },
    ]);
    const disk = buildChartGroups(FULL).find((g) => g.id === 'disk')?.charts[0];
    expect(disk?.series).toEqual([
      { key: 'disk.used:/', label: '/', slot: 0 },
      { key: 'disk.used:/srv', label: '/srv', slot: 1 },
    ]);
  });

  it('splits more than 8 labelled series into small multiples', () => {
    const sensors = Array.from({ length: 10 }, (_, i) => info(`temp:core${i}`, 'celsius'));
    const temps = buildChartGroups(sensors).find((g) => g.id === 'temps');
    expect(temps?.charts.map((c) => [c.title, c.series.length])).toEqual([
      ['Temperatures (1 of 2)', 8],
      ['Temperatures (2 of 2)', 2],
    ]);
    expect(temps?.charts[1]?.series[0]?.slot).toBe(0);
  });

  it('returns nothing for an empty series list', () => {
    expect(buildChartGroups([])).toEqual([]);
  });
});

describe('buildQuery', () => {
  it('asks for the range ending now with the default point count', () => {
    expect(buildQuery(['net.rx:*'], rangeById('7d'), 1_794_200_000.5)).toEqual({
      keys: ['net.rx:*'],
      from: 1_794_200_000 - 7 * 86_400,
      to: 1_794_200_000,
      points: 300,
    });
  });

  it('accepts a point count, e.g. for sparklines', () => {
    expect(buildQuery(['cpu.usage'], rangeById('1h'), 1000_000, 60).points).toBe(60);
  });
});
