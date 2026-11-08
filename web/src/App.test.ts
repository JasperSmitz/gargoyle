// @vitest-environment jsdom
import { render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App.svelte';
import { ApiRequestError } from './lib/api/client';
import { fixtures } from './lib/api/fixtures';
import type { Overview } from './lib/api/types';
import type { Visibility } from './lib/poller';

const visible: Visibility = { isHidden: () => false, subscribe: () => () => {} };

function clientWith(overview: (opts?: { signal?: AbortSignal }) => Promise<Overview>) {
  return { overview: vi.fn(overview) };
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

  it('routes unknown hashes to a not-found page', async () => {
    location.hash = '#/nowhere';
    const client = clientWith(async () => fixtures['overview.json']);
    render(App, { client, visibility: visible });
    expect(screen.getByText('Page not found')).toBeTruthy();
  });
});
