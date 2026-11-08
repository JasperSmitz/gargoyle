import { describe, expect, it } from 'vitest';
import { axisIncrements, formatAxisTicks, formatValue, yRange } from './units';

const MiB = 1024 ** 2;
const GiB = 1024 ** 3;

describe('formatValue', () => {
  it.each([
    ['percent', 12.46, '12%'],
    ['percent', 0.42, '0.4%'],
    ['bytes', 1.5 * GiB, '1.5 GiB'],
    ['bytes_per_s', 120_400, '118 KiB/s'],
    ['bytes_per_s', 0, '0 B/s'],
    ['celsius', 48, '48.0 °C'],
    ['load', 0.4, '0.40'],
    ['ms', 3.14, '3.1 ms'],
    ['ms', 42.6, '43 ms'],
    ['ms', 1530, '1.53 s'],
  ] as const)('%s %d → %s', (unit, value, text) => {
    expect(formatValue(unit, value)).toBe(text);
  });

  it.each(['percent', 'bytes', 'celsius', 'ms'] as const)('%s null is a placeholder', (unit) => {
    expect(formatValue(unit, null)).toBe('—');
    expect(formatValue(unit, NaN)).toBe('—');
  });
});

describe('formatAxisTicks', () => {
  it('labels percent ticks without float noise', () => {
    expect(formatAxisTicks('percent', [0, 25, 50.00000001])).toEqual(['0%', '25%', '50%']);
  });

  it('uses one IEC unit per byte axis, chosen by the largest tick', () => {
    expect(formatAxisTicks('bytes', [0, 512 * MiB, GiB, 1.5 * GiB])).toEqual([
      '0 GiB',
      '0.5 GiB',
      '1 GiB',
      '1.5 GiB',
    ]);
    expect(formatAxisTicks('bytes_per_s', [0, 50 * 1024, 100 * 1024])).toEqual([
      '0 KiB/s',
      '50 KiB/s',
      '100 KiB/s',
    ]);
  });

  it('switches ms axes to seconds from 1000 ms', () => {
    expect(formatAxisTicks('ms', [0, 5, 10])).toEqual(['0 ms', '5 ms', '10 ms']);
    expect(formatAxisTicks('ms', [0, 500, 1000, 1500])).toEqual(['0 s', '0.5 s', '1 s', '1.5 s']);
  });

  it('labels load and celsius plainly', () => {
    expect(formatAxisTicks('load', [0, 0.1 + 0.2, 1])).toEqual(['0', '0.3', '1']);
    expect(formatAxisTicks('celsius', [40, 50])).toEqual(['40 °C', '50 °C']);
  });
});

describe('axisIncrements', () => {
  it('steps byte axes in binary multiples and leaves other units to uPlot', () => {
    const incrs = axisIncrements('bytes') ?? [];
    expect(incrs).toContain(500 * MiB);
    expect(incrs).not.toContain(500 * 1000 * 1000);
    expect(incrs).toContain(GiB);
    expect(axisIncrements('percent')).toBeUndefined();
  });
});

describe('yRange', () => {
  it('keeps percent at 0–100 whatever the data', () => {
    expect(yRange('percent', 3, 12)).toEqual([0, 100]);
    expect(yRange('percent', null, null)).toEqual([0, 100]);
  });

  it('starts quantities at zero with headroom and a minimum extent', () => {
    expect(yRange('load', 0.2, 2)).toEqual([0, 2.2]);
    expect(yRange('load', 0, 0.1)).toEqual([0, 1]);
    expect(yRange('bytes_per_s', null, null)).toEqual([0, 1024]);
  });

  it('frames temperatures in 10-degree steps around the data', () => {
    expect(yRange('celsius', 41, 63)).toEqual([30, 70]);
    expect(yRange('celsius', 2, 8)).toEqual([0, 20]);
  });
});
