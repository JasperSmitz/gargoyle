// @vitest-environment jsdom
import { render, screen, within } from '@testing-library/svelte';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiRequestError, type ApiClient } from '../lib/api/client';
import { fixtures } from '../lib/api/fixtures';
import type { MetricsQuery, ServiceDetail } from '../lib/api/types';
import { rangeById } from '../lib/charts/range';
import type { Visibility } from '../lib/poller';
import { installResizeObserver } from '../test/fake-uplot';
import ServicePage from './ServicePage.svelte';

vi.mock('uplot', () => import('../test/fake-uplot'));
beforeAll(installResizeObserver);

const NOW = 1_794_200_000;
const visible: Visibility = { isHidden: () => false, subscribe: () => () => {} };
const failedExample = fixtures['service-detail.json'];

const memoryResponse = (key: string): MetricsQuery => ({
  host: 'debian-box',
  from: NOW - 86_400,
  to: NOW,
  step: 300,
  resolution: '5m',
  series: [
    {
      key,
      unit: 'bytes',
      points: [
        [NOW - 600, 150e6, 140e6, 160e6],
        [NOW - 300, 152e6, 150e6, 155e6],
      ],
    },
  ],
});

async function renderPage(
  detail: () => Promise<ServiceDetail>,
  metricsQuery: ApiClient['metricsQuery'] = async (q) => memoryResponse(q.keys[0] ?? ''),
  unit = 'minecraft.service',
) {
  const client = { service: vi.fn(detail), metricsQuery: vi.fn(metricsQuery) };
  render(ServicePage, {
    unit,
    range: rangeById('24h'),
    client,
    visibility: visible,
    serverNow: NOW,
  });
  await vi.advanceTimersByTimeAsync(0);
  return client;
}

function facts(): string {
  const summary = screen.getByRole('region', { name: 'Service summary' });
  return summary.querySelector('dl')?.textContent?.replace(/\s+/g, ' ') ?? '';
}

describe('ServicePage', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW * 1000);
  });
  afterEach(() => vi.useRealTimers());

  it('summarises a failed service: state, how long, restarts, memory', async () => {
    await renderPage(async () => failedExample);
    const summary = screen.getByRole('region', { name: 'Service summary' });
    expect(within(summary).getByRole('heading', { level: 1 }).textContent).toBe('Minecraft');
    expect(summary.textContent).toContain('unit failed (Result=exit-code)');
    expect(facts()).toContain('State failed (failed)');
    expect(facts()).toContain('Failed for 1h 23m');
    expect(facts()).toContain('Restarts 3');
    expect(facts()).toContain('Memory Not reported');
    expect(facts()).toContain('Description Minecraft Server');
  });

  it('shows uptime for a running service', async () => {
    const running = structuredClone(fixtures['services.json'].services[0]!);
    await renderPage(async () => ({ host: 'debian-box', service: running, events: [] }));
    expect(facts()).toContain('Up for 2d 7h');
    expect(facts()).toContain('Memory 145 MiB');
    expect(facts()).toContain('Restarts 0');
  });

  it("charts the service's memory and restarts over the selected range", async () => {
    const client = await renderPage(async () => failedExample);
    const window = { from: NOW - 86_400, to: NOW, points: 300 };
    expect(client.metricsQuery).toHaveBeenCalledWith(
      { keys: ['service.memory:minecraft.service'], ...window },
      expect.anything(),
    );
    expect(client.metricsQuery).toHaveBeenCalledWith(
      { keys: ['service.restarts:minecraft.service'], ...window },
      expect.anything(),
    );
    const memory = screen.getByRole('region', { name: 'History' });
    const titles = within(memory)
      .getAllByRole('figure')
      .map((f) => f.querySelector('figcaption span')?.textContent);
    expect(titles).toEqual(['Memory use', 'Restarts']);
    const picker = within(memory).getByRole('navigation', { name: 'Time range' });
    expect(within(picker).getByRole('link', { name: '7d' }).getAttribute('href')).toBe(
      '#/services/minecraft.service?range=7d',
    );
  });

  it('shows "no data" rather than an error for a series not recorded yet', async () => {
    await renderPage(
      async () => failedExample,
      async (q) => {
        throw new ApiRequestError('unknown_series', `unknown series key: ${q.keys[0]}`, 400);
      },
    );
    const history = screen.getByRole('region', { name: 'History' });
    expect(within(history).getAllByText('No data in this range')).toHaveLength(2);
    expect(within(history).queryByRole('alert')).toBeNull();
  });

  it.each([
    ['internal', 'history query failed', 500],
    ['bad_request', 'unknown series key: x', 400],
  ])('still reports other chart failures (%s)', async (code, message, status) => {
    await renderPage(
      async () => failedExample,
      async () => {
        throw new ApiRequestError(code, message, status);
      },
    );
    const history = screen.getByRole('region', { name: 'History' });
    expect(within(history).getAllByRole('alert')[0]?.textContent).toContain(message);
  });

  it('lists the service events', async () => {
    await renderPage(async () => failedExample);
    const events = screen.getByRole('region', { name: /^Events/ });
    expect(within(events).getAllByRole('listitem')).toHaveLength(1);
    expect(events.textContent).toContain('unit failed (Result=exit-code)');
  });

  it('shows not found for an unknown unit', async () => {
    await renderPage(
      async () => {
        throw new ApiRequestError('not_found', 'unknown service', 404);
      },
      undefined,
      'ghost.service',
    );
    expect(screen.getByText('No service called ghost.service')).toBeTruthy();
    expect(screen.queryByRole('region', { name: 'History' })).toBeNull();
  });
});
