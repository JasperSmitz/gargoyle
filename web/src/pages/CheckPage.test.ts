// @vitest-environment jsdom
import { render, screen, within } from '@testing-library/svelte';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiRequestError, type ApiClient } from '../lib/api/client';
import { fixtures } from '../lib/api/fixtures';
import type { CheckDetail, MetricsQuery } from '../lib/api/types';
import { rangeById } from '../lib/charts/range';
import type { Visibility } from '../lib/poller';
import { installResizeObserver } from '../test/fake-uplot';
import CheckPage from './CheckPage.svelte';

vi.mock('uplot', () => import('../test/fake-uplot'));
beforeAll(installResizeObserver);

const NOW = 1_794_200_000;
const visible: Visibility = { isHidden: () => false, subscribe: () => () => {} };
const failing = fixtures['check-detail.json'];
const healthy: CheckDetail = {
  host: 'debian-box',
  check: structuredClone(fixtures['checks.json'].checks[0]!),
  events: [],
};

const latencyResponse = (key: string): MetricsQuery => ({
  host: 'debian-box',
  from: NOW - 86_400,
  to: NOW,
  step: 300,
  resolution: '5m',
  series: [
    {
      key,
      unit: 'ms',
      points: [
        [NOW - 600, 3.2, 2.9, 4.1],
        [NOW - 300, 3.0, 2.8, 3.4],
      ],
    },
  ],
});

async function renderPage(
  detail: () => Promise<CheckDetail>,
  metricsQuery: ApiClient['metricsQuery'] = async (q) => latencyResponse(q.keys[0] ?? ''),
  name = 'Game port',
) {
  // Every key the page charts is listed, unless a test says otherwise.
  const listed = [`check.latency:${name}`];
  const metrics = vi.fn(async () => ({
    host: 'debian-box',
    series: listed.map((key) => ({
      key,
      metric: key.split(':')[0] ?? key,
      label: null,
      unit: 'ms' as const,
    })),
  }));
  const client = { check: vi.fn(detail), metricsQuery: vi.fn(metricsQuery), metrics };
  render(CheckPage, { name, range: rangeById('24h'), client, visibility: visible, serverNow: NOW });
  await vi.advanceTimersByTimeAsync(0);
  return client;
}

function summary(): HTMLElement {
  return screen.getByRole('region', { name: /check summary$/ });
}

function facts(): string {
  return summary().querySelector('dl')?.textContent?.replace(/\s+/g, ' ') ?? '';
}

describe('CheckPage', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW * 1000);
  });
  afterEach(() => vi.useRealTimers());

  it('summarises a failing check with its message, target and failure streak', async () => {
    const client = await renderPage(async () => failing);
    expect(client.check).toHaveBeenCalledWith('Game port', expect.anything());
    expect(summary().getAttribute('aria-label')).toBe('TCP check summary');
    expect(within(summary()).getByRole('heading', { level: 1 }).textContent).toBe('Game port');
    expect(within(summary()).getByText('Warning')).toBeTruthy();
    expect(summary().textContent).toContain('connection refused (1 consecutive failure)');
    expect(facts()).toContain('Last check 10s ago');
    expect(facts()).toContain('Latency Failed');
    expect(facts()).toContain('Consecutive failures 1 in a row');
    // The sanitized target sits under the name, like a unit name.
    expect(within(summary()).getByText('127.0.0.1:25565').tagName).toBe('CODE');
    // Long targets wrap between segments, not mid-word, and the text is unchanged.
    const code = within(summary()).getByText('127.0.0.1:25565');
    expect(code.querySelectorAll('wbr').length).toBeGreaterThan(0);
  });

  it('shows latency for a healthy check and no message', async () => {
    await renderPage(async () => healthy, undefined, 'Music HTTP');
    expect(facts()).toContain('Latency 3.1 ms');
    expect(facts()).toContain('Consecutive failures 0');
    expect(summary().querySelector('.message')).toBeNull();
  });

  it('says a check has not run yet', async () => {
    const pending = structuredClone(healthy);
    Object.assign(pending.check, {
      health: 'unknown',
      message: 'not yet run',
      lastCheck: null,
      latencyMs: null,
    });
    await renderPage(async () => pending, undefined, 'Music HTTP');
    expect(summary().textContent).toContain('not yet run');
    expect(facts()).toContain('Last check Not run yet');
    expect(facts()).toContain('Latency Not run yet');
  });

  it('charts the latency series over the selected range, without a threshold line', async () => {
    const client = await renderPage(async () => healthy, undefined, 'Music HTTP');
    expect(client.metricsQuery).toHaveBeenCalledWith(
      { keys: ['check.latency:Music HTTP'], from: NOW - 86_400, to: NOW, points: 300 },
      expect.anything(),
    );
    const history = screen.getByRole('region', { name: 'History' });
    const titles = within(history)
      .getAllByRole('figure')
      .map((f) => f.querySelector('figcaption span')?.textContent);
    expect(titles).toEqual(['Latency']);
    const picker = within(history).getByRole('navigation', { name: 'Time range' });
    expect(within(picker).getByRole('link', { name: '7d' }).getAttribute('href')).toBe(
      '#/checks/Music%20HTTP?range=7d',
    );
  });

  it('shows "no data" for a check that has never answered', async () => {
    await renderPage(
      async () => failing,
      async (q) => {
        throw new ApiRequestError('unknown_series', `unknown series key: ${q.keys[0]}`, 400);
      },
    );
    const history = screen.getByRole('region', { name: 'History' });
    expect(within(history).getByText('No data in this range')).toBeTruthy();
    expect(within(history).queryByRole('alert')).toBeNull();
  });

  it('does not query a latency series the backend has not listed yet', async () => {
    const metricsQuery = vi.fn(async (q: Parameters<ApiClient['metricsQuery']>[0]) =>
      latencyResponse(q.keys[0] ?? ''),
    );
    const client = {
      check: vi.fn(async () => failing),
      metricsQuery,
      metrics: vi.fn(async () => ({ host: 'debian-box', series: [] })),
    };
    render(CheckPage, {
      name: 'Game port',
      range: rangeById('24h'),
      client,
      visibility: visible,
      serverNow: NOW,
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(client.metrics).toHaveBeenCalled();
    expect(metricsQuery).not.toHaveBeenCalled();
    const history = screen.getByRole('region', { name: 'History' });
    expect(within(history).getByText('No data in this range')).toBeTruthy();
  });

  it('lists the check events and links to the filtered log', async () => {
    await renderPage(async () => failing);
    const events = screen.getByRole('region', { name: /^Events/ });
    expect(within(events).getAllByRole('listitem')).toHaveLength(1);
    expect(events.textContent).toMatch(/Healthy\s*→\s*Warning/);
    expect(
      within(events).getByRole('link', { name: 'All events for this check' }).getAttribute('href'),
    ).toBe('#/events?kind=check&target=Game+port');
  });

  it('has an empty state when no events were recorded', async () => {
    await renderPage(async () => healthy, undefined, 'Music HTTP');
    const events = screen.getByRole('region', { name: /^Events/ });
    expect(events.textContent).toContain('No health changes recorded');
  });

  it('shows not found for an unknown check', async () => {
    await renderPage(
      async () => {
        throw new ApiRequestError('not_found', 'unknown check', 404);
      },
      undefined,
      'Ghost',
    );
    expect(screen.getByText('No check called Ghost')).toBeTruthy();
    expect(screen.queryByRole('region', { name: 'History' })).toBeNull();
  });
});
