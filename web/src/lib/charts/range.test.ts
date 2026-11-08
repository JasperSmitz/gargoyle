import { describe, expect, it } from 'vitest';
import { DEFAULT_RANGE, RANGES, parseRange, rangeById, rangeWindow } from './range';

describe('ranges', () => {
  it('offers exactly the picker ranges, in order', () => {
    expect(RANGES.map((r) => r.id)).toEqual(['1h', '6h', '24h', '7d', '30d', '1y']);
  });

  it.each([
    ['1h', 3600],
    ['6h', 21_600],
    ['24h', 86_400],
    ['7d', 604_800],
    ['30d', 2_592_000],
    ['1y', 31_536_000],
  ] as const)('%s ends now and spans %d seconds', (id, seconds) => {
    expect(rangeWindow(rangeById(id), 1_794_200_000.9)).toEqual({
      from: 1_794_200_000 - seconds,
      to: 1_794_200_000,
    });
  });

  it.each([null, undefined, '', '2h', '7D'])('falls back to the default for %j', (value) => {
    expect(parseRange(value).id).toBe(DEFAULT_RANGE);
  });

  it('parses a known id', () => {
    expect(parseRange('30d').id).toBe('30d');
  });
});
