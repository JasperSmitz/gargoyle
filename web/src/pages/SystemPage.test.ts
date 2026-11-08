// @vitest-environment jsdom
import { render, screen } from '@testing-library/svelte';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiRequestError, type ApiClient } from '../lib/api/client';
import { fixtures } from '../lib/api/fixtures';
import type { MetricsResponse } from '../lib/api/types';
import { rangeById } from '../lib/charts/range';
import type { Visibility } from '../lib/poller';
import { installResizeObserver } from '../test/fake-uplot';
import SystemPage from './SystemPage.svelte';

vi.mock('uplot', () => import('../test/fake-uplot'));

beforeAll(installResizeObserver);

const visible: Visibility = { isHidden: () => false, subscribe: () => () => {} };

function clientWith(series: MetricsResponse = fixtures['metrics.json']) {
  return {
    metrics: vi.fn(async () => series),
    metricsQuery: vi.fn(
      async (_query: Parameters<ApiClient['metricsQuery']>[0]) => fixtures['metrics-query.json'],
    ),
  };
}

function chartTitles(): string[] {
  return screen
    .getAllByRole('figure')
    .map((f) => f.querySelector('figcaption span')?.textContent ?? '');
}

describe('SystemPage', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(1_794_200_000_000);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('renders one chart per metric the backend lists, with one query per group', async () => {
    const client = clientWith();
    render(SystemPage, { range: rangeById('24h'), client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    expect(chartTitles()).toEqual([
      'CPU',
      'Load average',
      'Memory and swap',
      'Disk usage',
      'Network enp1s0',
      'Temperatures',
    ]);
    expect(client.metricsQuery).toHaveBeenCalledTimes(6);
    expect(client.metricsQuery.mock.calls[0]?.[0]).toEqual({
      keys: ['cpu.usage'],
      from: 1_794_200_000 - 86_400,
      to: 1_794_200_000,
      points: 300,
    });
  });

  it('hides temperatures entirely when /metrics has no temp series', async () => {
    const noTemps = structuredClone(fixtures['metrics.json']);
    noTemps.series = noTemps.series.filter((s) => s.metric !== 'temp');
    render(SystemPage, {
      range: rangeById('1h'),
      client: clientWith(noTemps),
      visibility: visible,
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(chartTitles()).not.toContain('Temperatures');
    expect(screen.queryByText(/temperature/i)).toBeNull();
  });

  it('refreshes every 60s and immediately when the range changes', async () => {
    const client = clientWith();
    const { rerender } = render(SystemPage, {
      range: rangeById('1h'),
      client,
      visibility: visible,
    });
    await vi.advanceTimersByTimeAsync(0);
    const perPoll = client.metricsQuery.mock.calls.length;

    await vi.advanceTimersByTimeAsync(60_000);
    expect(client.metricsQuery.mock.calls.length).toBe(perPoll * 2);

    await rerender({ range: rangeById('30d'), client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    expect(client.metricsQuery.mock.calls.length).toBe(perPoll * 3);
    const last = client.metricsQuery.mock.calls.at(-1)?.[0];
    expect((last?.to ?? 0) - (last?.from ?? 0)).toBe(30 * 86_400);
    expect(screen.getByRole('link', { name: '30d' }).getAttribute('aria-current')).toBe('true');
  });

  it('puts the range picker in one row of links that keep the choice in the URL', async () => {
    render(SystemPage, { range: rangeById('6h'), client: clientWith(), visibility: visible });
    const picker = screen.getByRole('navigation', { name: 'Time range' });
    const links = [...picker.querySelectorAll('a')];
    expect(links.map((a) => a.textContent)).toEqual(['1h', '6h', '24h', '7d', '30d', '1y']);
    expect(links[3]?.getAttribute('href')).toBe('#/system?range=7d');
  });

  it('shows a per-chart error when one group fails, leaving the others', async () => {
    const client = clientWith();
    client.metricsQuery.mockImplementation(async (query) => {
      if (query.keys.includes('cpu.usage')) throw new ApiRequestError('internal', 'db locked', 500);
      return fixtures['metrics-query.json'];
    });
    render(SystemPage, { range: rangeById('1h'), client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    const alerts = screen.getAllByRole('alert');
    expect(alerts).toHaveLength(1);
    expect(alerts[0]?.textContent).toContain('db locked');
  });

  it('says so when the series list cannot be loaded', async () => {
    const client = clientWith();
    client.metrics.mockRejectedValue(new ApiRequestError('network', 'Failed to fetch', 0));
    render(SystemPage, { range: rangeById('1h'), client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.getByText('Could not load the list of metrics')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Try again' })).toBeTruthy();
  });
});
