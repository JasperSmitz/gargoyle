// @vitest-environment jsdom
import { render, screen, within } from '@testing-library/svelte';
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
    library: vi.fn(async () => fixtures['library.json']),
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

  it('shows a meaningful shell, not a skeleton, when the first load fails', async () => {
    const client = clientWith(async () => {
      throw new ApiRequestError('network', 'Failed to fetch', 0);
    });
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.getByRole('alert').textContent).toContain('Server unreachable');
    expect(screen.queryByRole('status', { name: 'Loading overview' })).toBeNull();
    expect(screen.getByRole('link', { name: 'Gargoyle overview' })).toBeTruthy();
    expect(screen.getByRole('heading', { level: 1, name: 'Waiting for the server' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Retry now' })).toBeTruthy();
    expect(document.title).toBe('Offline · Gargoyle');
  });

  it('asks for a reload when any request answers 401', async () => {
    location.hash = '#/timers/music-sync.timer';
    const client = clientWith(async () => fixtures['overview.json']);
    client.timer.mockRejectedValue(new ApiRequestError('unauthorized', 'auth required', 401));
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.getAllByRole('alert')[0]?.textContent).toContain(
      'Authentication required — reload to sign in',
    );
    expect(screen.getByRole('button', { name: 'Reload' })).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Retry now' })).toBeNull();
    expect(document.title).toBe('Sign-in needed · music-sync.timer · Gargoyle — debian-box');
  });

  it('shows the authentication message instead of the offline banner on a first-load 401', async () => {
    const client = clientWith(async () => {
      throw new ApiRequestError('unauthorized', 'auth required', 401);
    });
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.getByRole('alert').textContent).toContain('Authentication required');
    expect(screen.queryByText('Server error')).toBeNull();
    expect(screen.getByRole('heading', { level: 1, name: 'Sign-in needed' })).toBeTruthy();
    expect(document.title).toBe('Sign-in needed · Gargoyle');
  });

  it('moves focus to the new page heading on navigation, and keeps it on query changes', async () => {
    const client = clientWith(async () => fixtures['overview.json']);
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    const navigate = async (to: string) => {
      location.hash = to;
      window.dispatchEvent(new HashChangeEvent('hashchange'));
      await vi.advanceTimersByTimeAsync(0);
    };

    await navigate('#/system');
    expect(document.activeElement).toBe(screen.getByRole('heading', { level: 1, name: 'System' }));
    expect(document.title).toContain('System · Gargoyle');

    const range = screen.getByRole('link', { name: '7d' });
    range.focus();
    await navigate('#/system?range=7d');
    expect(document.activeElement).toBe(range);
  });

  it.each([
    ['service', '#/services/minecraft.service', 'minecraft.service', 'Minecraft'],
    ['timer', '#/timers/music-sync.timer', 'music-sync.timer', 'Music sync'],
    ['check', '#/checks/Game%20port', 'Game port', 'Game port'],
  ] as const)(
    'keeps route focus on the %s heading when its detail arrives',
    async (kind, to, loadingName, loadedName) => {
      const client = clientWith(async () => fixtures['overview.json']);
      let resolve: () => void = () => {};
      const pending = new Promise<void>((r) => (resolve = r));
      const fixture = {
        service: 'service-detail.json',
        timer: 'timer-detail.json',
        check: 'check-detail.json',
      } as const;
      client[kind].mockImplementation((async () => {
        await pending;
        return fixtures[fixture[kind]];
      }) as never);
      render(App, { client, visibility: visible });
      await vi.advanceTimersByTimeAsync(0);

      location.hash = to;
      window.dispatchEvent(new HashChangeEvent('hashchange'));
      await vi.advanceTimersByTimeAsync(0);
      const heading = screen.getByRole('heading', { level: 1, name: loadingName });
      expect(document.activeElement).toBe(heading);

      resolve();
      await vi.advanceTimersByTimeAsync(0);
      expect(screen.getByRole('heading', { level: 1, name: loadedName })).toBe(heading);
      expect(document.activeElement).toBe(heading);
    },
  );

  it('keeps a heading on a detail page that could not load', async () => {
    location.hash = '#/services/minecraft.service';
    const client = clientWith(async () => fixtures['overview.json']);
    client.service.mockRejectedValue(new ApiRequestError('network', 'Failed to fetch', 0));
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.getByRole('heading', { level: 1, name: 'minecraft.service' })).toBeTruthy();
    expect(screen.getByText('Could not load this service')).toBeTruthy();
  });

  it('restores focus to an events filter that the page re-created', async () => {
    location.hash = '#/events';
    const client = clientWith(async () => fixtures['overview.json']);
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    screen.getByRole('combobox', { name: 'Kind' }).focus();
    location.hash = '#/events?kind=service';
    window.dispatchEvent(new HashChangeEvent('hashchange'));
    await vi.advanceTimersByTimeAsync(0);
    const kind = screen.getByRole('combobox', { name: 'Kind' }) as HTMLSelectElement;
    expect(document.activeElement).toBe(kind);
  });

  it('has a skip link that focuses the page heading without changing the route', async () => {
    const client = clientWith(async () => fixtures['overview.json']);
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    screen.getByRole('link', { name: 'Skip to content' }).click();
    expect(location.hash).toBe('');
    expect(document.activeElement?.tagName).toBe('H1');
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
    expect(document.title).toBe('System · Gargoyle');
    await vi.advanceTimersByTimeAsync(0);
    // The example overview is critical with two critical targets.
    expect(document.title).toBe('✖ 2 critical · System · Gargoyle — debian-box');
    expect(icon()).toBe(faviconHref('critical'));

    const healthy = structuredClone(fixtures['overview.json']);
    healthy.health = 'healthy';
    client.overview.mockResolvedValue(healthy);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(document.title).toBe('System · Gargoyle — debian-box');
    expect(icon()).toBe(faviconHref('healthy'));

    client.overview.mockRejectedValue(new ApiRequestError('network', 'Failed to fetch', 0));
    await vi.advanceTimersByTimeAsync(10_000);
    expect(document.title).toBe('Offline · System · Gargoyle — debian-box');
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

  it('shows the Library nav entry only when the library is configured', async () => {
    const client = clientWith(async () => fixtures['overview.json']);
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    const nav = screen.getByRole('navigation', { name: 'Main' });
    expect(
      within(nav)
        .getAllByRole('link')
        .map((a) => a.textContent),
    ).toEqual(['Overview', 'System', 'Library', 'Events']);
    expect(within(nav).getByRole('link', { name: 'Library' }).getAttribute('href')).toBe(
      '#/library',
    );

    client.overview.mockResolvedValue({ ...fixtures['overview.json'], library: null });
    await vi.advanceTimersByTimeAsync(10_000);
    expect(within(nav).queryByRole('link', { name: 'Library' })).toBeNull();
  });

  it('routes #/library with the severity filter and marks the nav entry', async () => {
    location.hash = '#/library?severity=error';
    const client = clientWith(async () => fixtures['overview.json']);
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    expect(client.library).toHaveBeenCalled();
    expect(screen.getByRole('heading', { level: 1, name: 'Music library' })).toBeTruthy();
    expect(screen.getByRole('link', { name: 'Library' }).getAttribute('aria-current')).toBe('page');
    expect(screen.getByRole('link', { name: 'Errors only' }).getAttribute('aria-current')).toBe(
      'true',
    );
    expect(document.title).toContain('Library');
  });

  it('explains a direct visit to #/library without [library]', async () => {
    location.hash = '#/library';
    const client = clientWith(async () => ({ ...fixtures['overview.json'], library: null }));
    client.library.mockResolvedValue(fixtures['library-unconfigured.json']);
    render(App, { client, visibility: visible });
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.getByText('No music library configured')).toBeTruthy();
    expect(screen.queryByRole('link', { name: 'Library' })).toBeNull();
  });
});
