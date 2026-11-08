import { describe, expect, it } from 'vitest';
import { formatChartTime, formatTimeTicks } from './time';

const at = (d: number, h: number, m = 0) => new Date(2026, 10, d, h, m).getTime() / 1000;

describe('formatChartTime', () => {
  it('prints a bare 24-hour time on the newest bucket’s day', () => {
    expect(formatChartTime(at(8, 7, 30), at(8, 9))).toBe(
      new Date(2026, 10, 8, 7, 30).toLocaleTimeString(undefined, {
        hour: '2-digit',
        minute: '2-digit',
        hourCycle: 'h23',
      }),
    );
  });

  it('adds the date for times on an earlier day', () => {
    const text = formatChartTime(at(7, 21, 30), at(8, 9));
    expect(text).toContain('21:30');
    expect(text).not.toBe(formatChartTime(at(8, 21, 30), at(8, 22)));
  });
});

describe('formatTimeTicks', () => {
  it('keeps time-of-day ticks to five characters and dates midnight', () => {
    const [nine, midnight] = formatTimeTicks([at(8, 9), at(9, 0)], 3 * 3600);
    expect(nine).toBe('09:00');
    expect(midnight).not.toContain(':');
  });
});
