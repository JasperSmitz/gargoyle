// @vitest-environment jsdom
import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { fixtures } from '../../lib/api/fixtures';
import { alignSeries } from '../../lib/charts/data';
import type { ChartSeries } from '../../lib/charts/groups';
import { installResizeObserver, plots } from '../../test/fake-uplot';
import TimeSeriesChart from './TimeSeriesChart.svelte';

// The uPlot wiring itself is covered by plot-options.test.ts.
vi.mock('uplot', () => import('../../test/fake-uplot'));

beforeAll(installResizeObserver);

beforeEach(() => {
  plots.length = 0;
});

const TWO: ChartSeries[] = [
  { key: 'cpu.usage', label: 'Usage', slot: 0 },
  { key: 'mem.used', label: 'Memory', slot: 1 },
];
const example = fixtures['metrics-query.json'];

function props(overrides: Record<string, unknown> = {}) {
  return {
    title: 'CPU',
    unit: 'percent' as const,
    series: TWO,
    data: alignSeries(
      example,
      TWO.map((s) => s.key),
    ),
    rangeDescription: 'last hour',
    ...overrides,
  };
}

describe('TimeSeriesChart', () => {
  it('renders a plot with an accessible summary and hands uPlot the columns, nulls intact', () => {
    render(TimeSeriesChart, props());
    const figure = screen.getByRole('figure', { name: 'CPU' });
    const img = within(figure).getByRole('img');
    expect(img.getAttribute('aria-label')).toBe(
      'CPU, last hour. Usage: latest 11%, range 8.0% to 14%. Memory: latest 26%, range 26% to 26%.',
    );
    expect(plots).toHaveLength(1);
    expect(plots[0]?.data[1]).toEqual([10.2, null, 11.0]);
  });

  it('shows a legend for two or more series and none for one', () => {
    const { unmount } = render(TimeSeriesChart, props());
    const legend = screen.getByRole('list', { name: 'Legend' });
    expect(
      within(legend)
        .getAllByRole('listitem')
        .map((li) => li.textContent),
    ).toEqual(['Usage', 'Memory']);
    unmount();

    const one = TWO.slice(0, 1);
    render(TimeSeriesChart, props({ series: one, data: alignSeries(example, ['cpu.usage']) }));
    expect(screen.queryByRole('list', { name: 'Legend' })).toBeNull();
  });

  it('shows a loading state before the first response', () => {
    render(TimeSeriesChart, props({ data: null, loading: true }));
    expect(screen.getByRole('status', { name: /^Loading/ })).toBeTruthy();
    expect(plots).toHaveLength(0);
  });

  it('shows an empty state when the range has no values', () => {
    const empty = alignSeries(
      { ...example, series: [] },
      TWO.map((s) => s.key),
    );
    render(TimeSeriesChart, props({ data: empty }));
    expect(screen.getByText('No data in this range')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Show table' })).toBeNull();
    expect(plots).toHaveLength(0);
  });

  it('shows an error without data, and a note over existing data', async () => {
    const { rerender } = render(TimeSeriesChart, props({ data: null, error: 'boom' }));
    expect(screen.getByRole('alert').textContent).toContain('Could not load: boom');
    await rerender(props({ error: 'boom' }));
    expect(screen.getByRole('alert').textContent).toContain('Update failed: boom');
    expect(screen.getByRole('img')).toBeTruthy();
  });

  it('offers a table view of the data with gaps as placeholders', async () => {
    render(TimeSeriesChart, props());
    const toggle = screen.getByRole('button', { name: 'Show table' });
    await fireEvent.click(toggle);
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    const table = screen.getByRole('table');
    const rows = within(table).getAllByRole('row');
    expect(rows[0]?.textContent).toContain('Usage');
    expect(rows).toHaveLength(4);
    // min/max is reachable without hovering.
    expect(within(rows[1] as HTMLElement).getAllByRole('cell')[0]?.textContent).toContain(
      '(8.0% – 14%)',
    );
    expect(
      within(rows[2] as HTMLElement)
        .getAllByRole('cell')
        .map((c) => c.textContent?.trim()),
    ).toEqual(['—', '—']);
    expect(
      within(rows[1] as HTMLElement)
        .getAllByRole('cell')[1]
        ?.textContent?.trim(),
    ).toBe('26%');
  });

  it('updates data in place and rebuilds only when the series change', async () => {
    const { rerender } = render(TimeSeriesChart, props());
    await rerender(props({ series: TWO.map((s) => ({ ...s })) }));
    expect(plots).toHaveLength(1);
    expect(plots[0]?.setData).toHaveBeenCalled();

    await rerender(props({ series: [...TWO].reverse() }));
    expect(plots).toHaveLength(2);
    expect(plots[0]?.destroyed).toBe(true);
  });
});
