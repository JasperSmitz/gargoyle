import { describe, expect, it } from 'vitest';
import {
  PLACEHOLDER,
  formatBytes,
  formatDuration,
  formatLoad,
  formatPercent,
  formatRelativeTime,
  formatTimestamp,
} from './format';

describe('formatBytes', () => {
  it.each([
    [0, '0 B'],
    [512, '512 B'],
    [1023, '1023 B'],
    [1024, '1.0 KiB'],
    [1536, '1.5 KiB'],
    [152_000_000, '145 MiB'],
    [4_271_000_000, '4.0 GiB'],
    [16_624_951_296, '15.5 GiB'],
    [250_000_000_000, '233 GiB'],
    [99.97 * 1024 ** 3, '100 GiB'],
    [99.94 * 1024 ** 3, '99.9 GiB'],
    [1023.6 * 1024 ** 3, '1.0 TiB'],
    [1024 ** 3 - 1, '1.0 GiB'],
    [2_000_000_000_000, '1.8 TiB'],
    [1024 ** 6, '1024 PiB'],
  ])('%d -> %s', (bytes, expected) => {
    expect(formatBytes(bytes)).toBe(expected);
  });

  it.each([null, -1, Number.NaN, Number.POSITIVE_INFINITY])('%s -> placeholder', (value) => {
    expect(formatBytes(value)).toBe(PLACEHOLDER);
  });
});

describe('formatPercent', () => {
  it.each([
    [0, '0%'],
    [0.04, '0.0%'],
    [0.4, '0.4%'],
    [9.94, '9.9%'],
    [9.96, '10%'],
    [12.5, '13%'],
    [84, '84%'],
    [100, '100%'],
  ])('%d -> %s', (value, expected) => {
    expect(formatPercent(value)).toBe(expected);
  });

  it('uses the placeholder for missing values', () => {
    expect(formatPercent(null)).toBe(PLACEHOLDER);
    expect(formatPercent(Number.NaN)).toBe(PLACEHOLDER);
  });
});

describe('formatLoad', () => {
  it('uses two decimals', () => {
    expect(formatLoad(0.4)).toBe('0.40');
    expect(formatLoad(12.345)).toBe('12.35');
    expect(formatLoad(null)).toBe(PLACEHOLDER);
  });
});

describe('formatDuration', () => {
  it.each([
    [0, '0s'],
    [0.9, '0s'],
    [42, '42s'],
    [60, '1m'],
    [245, '4m 5s'],
    [3600, '1h'],
    [3660, '1h 1m'],
    [11_520, '3h 12m'],
    [86_400, '1d'],
    [90_061, '1d 1h'],
    [700_000, '8d 2h'],
  ])('%d -> %s', (seconds, expected) => {
    expect(formatDuration(seconds)).toBe(expected);
  });

  it('uses the placeholder for missing or negative values', () => {
    expect(formatDuration(null)).toBe(PLACEHOLDER);
    expect(formatDuration(-1)).toBe(PLACEHOLDER);
  });
});

describe('formatRelativeTime', () => {
  const now = 1_794_200_000;

  it.each([
    [now, 'just now'],
    [now - 4, 'just now'],
    [now + 4, 'just now'],
    [now - 42, '42s ago'],
    [now - 300, '5m ago'],
    [now - 3 * 3600 - 59, '3h ago'],
    [now - 2 * 86_400, '2d ago'],
    [now + 300, 'in 5m'],
    [now + 76_400, 'in 21h'],
  ])('%d -> %s', (ts, expected) => {
    expect(formatRelativeTime(ts, now)).toBe(expected);
  });

  it("uses the placeholder when there is no timestamp, including the backend's 0", () => {
    expect(formatRelativeTime(null, now)).toBe(PLACEHOLDER);
    expect(formatRelativeTime(0, now)).toBe(PLACEHOLDER);
  });
});

describe('formatTimestamp', () => {
  it('renders a non-empty local date for a timestamp', () => {
    expect(formatTimestamp(1_794_200_000)).toMatch(/2026/);
    expect(formatTimestamp(null)).toBe(PLACEHOLDER);
    expect(formatTimestamp(0)).toBe(PLACEHOLDER);
  });
});
