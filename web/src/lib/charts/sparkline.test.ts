import { describe, expect, it } from 'vitest';
import { sparklinePath } from './sparkline';

describe('sparklinePath', () => {
  it('lifts the pen over null buckets', () => {
    const { path, end } = sparklinePath([0, 10, null, 10, 0], 40, 10, 10);
    expect(path).toBe('M0 10L10 0M30 0L40 10');
    expect(end).toEqual({ x: 40, y: 10 });
  });

  it('draws nothing without values', () => {
    expect(sparklinePath([null, null], 40, 10, 1)).toEqual({ path: '', end: null });
    expect(sparklinePath([], 40, 10, 1)).toEqual({ path: '', end: null });
  });

  it('centres a flat series within the minimum span instead of amplifying jitter', () => {
    const { path } = sparklinePath([50, 50], 10, 10, 10);
    expect(path).toBe('M0 5L10 5');
  });
});
