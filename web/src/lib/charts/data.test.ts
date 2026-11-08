import { describe, expect, it } from 'vitest';
import { fixtures } from '../api/fixtures';
import type { MetricsQuery } from '../api/types';
import { alignSeries, averagesByKey, extent, hasValues, lastValue, toPlotColumns } from './data';

const example = fixtures['metrics-query.json'];

describe('alignSeries', () => {
  it('keeps null buckets as null so the chart draws gaps', () => {
    const data = alignSeries(example, ['cpu.usage', 'mem.used']);
    expect(data.ts).toEqual([1794196400, 1794196412, 1794196424]);
    expect(data.series[0]).toEqual({
      avg: [10.2, null, 11.0],
      min: [8.0, null, 9.5],
      max: [14.1, null, 12.7],
    });
    expect(data.series[1]?.avg).toEqual([25.6, null, 25.7]);
  });

  it('returns columns in the requested order, all-null for a key the response lacks', () => {
    const data = alignSeries(example, ['mem.used', 'swap.used']);
    expect(data.series[0]?.avg).toEqual([25.6, null, 25.7]);
    expect(data.series[1]).toEqual({
      avg: [null, null, null],
      min: [null, null, null],
      max: [null, null, null],
    });
  });

  it('merges series with different buckets on one sorted axis', () => {
    const response: MetricsQuery = {
      ...example,
      series: [
        { key: 'a', unit: 'load', points: [[20, 2, 2, 2]] },
        {
          key: 'b',
          unit: 'load',
          points: [
            [30, 3, 3, 3],
            [10, 1, 1, 1],
          ],
        },
      ],
    };
    const data = alignSeries(response, ['a', 'b']);
    expect(data.ts).toEqual([10, 20, 30]);
    expect(data.series[0]?.avg).toEqual([null, 2, null]);
    expect(data.series[1]?.avg).toEqual([1, null, 3]);
  });

  it('handles an empty response', () => {
    const data = alignSeries({ ...example, series: [] }, ['cpu.usage']);
    expect(data.ts).toEqual([]);
    expect(hasValues(data)).toBe(false);
  });
});

describe('toPlotColumns', () => {
  it('lays out [ts, avg, min, max] per series with nulls preserved', () => {
    const columns = toPlotColumns(alignSeries(example, ['cpu.usage', 'mem.used']));
    expect(columns).toHaveLength(7);
    expect(columns[0]).toEqual([1794196400, 1794196412, 1794196424]);
    expect(columns[1]).toEqual([10.2, null, 11.0]);
    expect(columns[2]).toEqual([8.0, null, 9.5]);
    expect(columns[3]).toEqual([14.1, null, 12.7]);
    expect(columns[4]).toEqual([25.6, null, 25.7]);
  });
});

describe('helpers', () => {
  it('finds the last value and the extent while skipping nulls', () => {
    expect(lastValue([1, 2, null])).toBe(2);
    expect(lastValue([null, null])).toBeNull();
    expect(
      extent([
        [3, null, 1],
        [null, 7],
      ]),
    ).toEqual({ min: 1, max: 7 });
    expect(extent([[null]])).toBeNull();
  });

  it('keys averages by series key for glob queries', () => {
    const byKey = averagesByKey(example);
    expect(byKey.get('mem.used')).toEqual([25.6, null, 25.7]);
    expect([...byKey.keys()]).toEqual(['cpu.usage', 'mem.used']);
  });
});
