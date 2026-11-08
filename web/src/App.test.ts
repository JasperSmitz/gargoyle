// @vitest-environment jsdom
import { render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App.svelte';
import { ApiRequestError, type ApiClient } from './lib/api/client';
import { fixtures } from './lib/api/fixtures';
import { faviconHref } from './lib/document-health';
import type { Overview } from './lib/api/types';
import type { Visibility } from './lib/poller';
import { installResizeObserver } from './test/fake-uplot';

vi.mock('uplot', () => import('./test/fake-uplot'));
installResizeObserver();

const visible: Visibility = { isHidden: () => false, subscribe: () => () => {} };

function clientWith(overview: (opts?: { signal?: AbortSignal }) => Promise<Overview>) {
  return {
    overview: vi.fn(overview),
    metrics: vi.fn(async () => fixtures['metrics.json']),
    metricsQuery: vi.fn(async (_query: Parameters<ApiClient['metricsQuery']>[0]) => {
      return fixtures['metrics-query.json'];
    }),
    service: vi.fn(async () => fixtures['service-detail.json']),
    timer: vi.fn(async () => fixtures['timer-detail.json']),
    check: vi.fn(async () => fixtures['check-detail.json']),
    events: vi.fn(async () => fixtures['events.json']),
  };
}

describe('App', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    location.hash = '';
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('shows a loading skeleton until the first overview arrives', async () => {
    let resolve: (o: Overview) => void = () => {};
    const client = clientWith(() => new Promise((r) => (resolve = r)));
    render(App, { client, visibility: visible });
    expect(screen.getByRole('status', { name: 'Loading overview' })).toBeTruthy();

    resolve(fixtures['overview.json']);
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.queryByRole('status', { name: 'Loading overview' })).toBeNull();
    expect(screen.getByText('debian-box')).toBeTruthy();
    expect(screen.getByText(/Updated/).textContent).toContain('just now');
  });

  it('keeps the last data and shows a stale banner when polling fails, then recovers', async () => {
    const client = clientWith(async () => fixtures['overview.json']);
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.queryByRole('alert')).toBeNull();

    client.overview.mockRejectedValueOnce(new ApiRequestError('network', 'Failed to fetch', 0));
    await vi.advanceTimersByTimeAsync(10_000);
    const alert = screen.getByRole('alert');
    expect(alert.textContent).toContain('Server unreachable');
    expect(screen.getByText(/Showing data from/).textContent).toContain('10s ago');
    expect(screen.getByRole('region', { name: 'System' })).toBeTruthy();

    await vi.advanceTimersByTimeAsync(10_000);
    expect(screen.queryByRole('alert')).toBeNull();
    expect(client.overview).toHaveBeenCalledTimes(3);
  });

  it('keeps the announced alert text static while the data age ticks', async () => {
    const client = clientWith(async () => fixtures['overview.json']);
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    client.overview.mockRejectedValue(new ApiRequestError('network', 'Failed to fetch', 0));
    await vi.advanceTimersByTimeAsync(10_000);

    const alert = screen.getByRole('alert');
    const announced = alert.textContent;
    const age = screen.getByText(/Showing data from/);
    const before = age.textContent;
    expect(alert.contains(age)).toBe(false);

    await vi.advanceTimersByTimeAsync(3_000);
    expect(screen.getByRole('alert').textContent).toBe(announced);
    expect(screen.getByText(/Showing data from/).textContent).not.toBe(before);
  });

  it('passes an abort signal to the client', async () => {
    const client = clientWith(async () => fixtures['overview.json']);
    render(App, { client, visibility: visible });
    expect(client.overview.mock.calls[0]?.[0]?.signal).toBeInstanceOf(AbortSignal);
  });

  it('shows the banner without a skeleton when the first load fails', async () => {
    const client = clientWith(async () => {
      throw new ApiRequestError('internal', 'boom', 500);
    });
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.getByRole('alert').textContent).toContain('Server error');
    expect(screen.queryByRole('status', { name: 'Loading overview' })).toBeNull();
  });

  it('loads last-hour sparklines on the overview only', async () => {
    const client = clientWith(async () => fixtures['overview.json']);
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    expect(client.metricsQuery).toHaveBeenCalledOnce();
    const query = client.metricsQuery.mock.calls[0]?.[0];
    expect(query?.keys).toEqual(['cpu.usage', 'load.1', 'mem.used', 'disk.used:*']);
    expect((query?.to ?? 0) - (query?.from ?? 0)).toBe(3600);
    expect(screen.getByRole('img', { name: /^CPU usage, last hour/ })).toBeTruthy();
  });

  it('routes #/system to the System page with the range from the URL and marks the nav', async () => {
    location.hash = '#/system?range=7d';
    const client = clientWith(async () => fixtures['overview.json']);
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.getByRole('heading', { level: 1, name: 'System' })).toBeTruthy();
    expect(screen.getByRole('link', { name: 'System' }).getAttribute('aria-current')).toBe('page');
    expect(screen.getByRole('link', { name: '7d' }).getAttribute('aria-current')).toBe('true');
    expect(client.metrics).toHaveBeenCalled();
  });

  it('routes unknown hashes to a not-found page', async () => {
    location.hash = '#/nowhere';
    const client = clientWith(async () => fixtures['overview.json']);
    render(App, { client, visibility: visible });
    expect(screen.getByText('Page not found')).toBeTruthy();
  });

  it('routes #/timers/:unit to the timer page and switches when the unit changes', async () => {
    location.hash = '#/timers/music-sync.timer';
    const client = clientWith(async () => fixtures['overview.json']);
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    expect(client.timer).toHaveBeenCalledWith('music-sync.timer', { limit: 50 }, expect.anything());
    expect(screen.getByRole('heading', { level: 1, name: 'Music sync' })).toBeTruthy();

    client.timer.mockRejectedValue(new ApiRequestError('not_found', 'unknown timer', 404));
    location.hash = '#/timers/other.timer';
    window.dispatchEvent(new HashChangeEvent('hashchange'));
    await vi.advanceTimersByTimeAsync(0);
    expect(client.timer).toHaveBeenLastCalledWith('other.timer', { limit: 50 }, expect.anything());
    expect(screen.getByText('No timer called other.timer')).toBeTruthy();
  });

  it('routes #/services/:unit to the service page', async () => {
    location.hash = '#/services/minecraft.service?range=7d';
    const client = clientWith(async () => fixtures['overview.json']);
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    expect(client.service).toHaveBeenCalledWith('minecraft.service', expect.anything());
    expect(screen.getByRole('heading', { level: 1, name: 'Minecraft' })).toBeTruthy();
    expect(screen.getByRole('link', { name: '7d' }).getAttribute('aria-current')).toBe('true');
  });

  it('routes #/checks/:name to the check page', async () => {
    location.hash = '#/checks/Game%20port?range=7d';
    const client = clientWith(async () => fixtures['overview.json']);
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    expect(client.check).toHaveBeenCalledWith('Game port', expect.anything());
    expect(screen.getByRole('heading', { level: 1, name: 'Game port' })).toBeTruthy();
    expect(screen.getByRole('link', { name: '7d' }).getAttribute('aria-current')).toBe('true');
  });

  it('mirrors overall health in the document title and favicon, on every page', async () => {
    location.hash = '#/system';
    const icon = () => document.querySelector('link[rel="icon"]')?.getAttribute('href');
    const client = clientWith(async () => fixtures['overview.json']);
    render(App, { client, visibility: visible });
    expect(document.title).toBe('Gargoyle');
    await vi.advanceTimersByTimeAsync(0);
    // The example overview is critical with two critical targets.
    expect(document.title).toBe('✖ 2 critical · Gargoyle — debian-box');
    expect(icon()).toBe(faviconHref('critical'));

    const healthy = structuredClone(fixtures['overview.json']);
    healthy.health = 'healthy';
    client.overview.mockResolvedValue(healthy);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(document.title).toBe('Gargoyle — debian-box');
    expect(icon()).toBe(faviconHref('healthy'));

    client.overview.mockRejectedValue(new ApiRequestError('network', 'Failed to fetch', 0));
    await vi.advanceTimersByTimeAsync(10_000);
    expect(document.title).toBe('Offline · Gargoyle — debian-box');
    expect(icon()).toBe(faviconHref('unknown'));
  });

  it('routes #/events with its filters and marks the nav entry', async () => {
    location.hash = '#/events?target=minecraft.service';
    const client = clientWith(async () => fixtures['overview.json']);
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.getByRole('link', { name: 'Events' }).getAttribute('aria-current')).toBe('page');
    expect(client.events).toHaveBeenCalledWith(
      { limit: 50, target: 'minecraft.service', kind: undefined },
      expect.anything(),
    );
  });
});
