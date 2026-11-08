// @vitest-environment jsdom
import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiRequestError, type ApiClient } from '../lib/api/client';
import { fixtures } from '../lib/api/fixtures';
import type { EventsResponse, HealthEvent } from '../lib/api/types';
import type { EventFilters } from '../lib/event-filters';
import type { Visibility } from '../lib/poller';
import EventsPage from './EventsPage.svelte';

const NOW = 1_794_200_000;
const visible: Visibility = { isHidden: () => false, subscribe: () => () => {} };

type EventsQuery = Parameters<ApiClient['events']>[0];

/** `n` events, one per minute back from NOW, alternating timer and service. */
function log(n: number): HealthEvent[] {
  return Array.from({ length: n }, (_, i) => ({
    id: 1000 - i,
    ts: NOW - i * 60,
    targetKind: i % 2 === 0 ? 'timer' : 'service',
    target: i % 2 === 0 ? 'music-sync.timer' : 'minecraft.service',
    from: 'healthy',
    to: 'critical',
    message: `event ${i}`,
  }));
}

/** A backend over `all`: exclusive `until`, whole seconds per page, target and kind filters. */
function backendOver(all: HealthEvent[]) {
  return vi.fn(async (query: EventsQuery = {}): Promise<EventsResponse> => {
    const matching = all.filter(
      (e) =>
        (query.until === undefined || e.ts < query.until) &&
        (query.target === undefined || e.target === query.target) &&
        (query.kind === undefined || e.targetKind === query.kind),
    );
    const limit = query.limit ?? 100;
    const last = matching[limit - 1];
    const events =
      last === undefined ? matching : matching.filter((e, i) => i < limit || e.ts === last.ts);
    return { host: 'debian-box', events };
  });
}

async function renderPage(
  events: ApiClient['events'] | ReturnType<typeof backendOver>,
  filters: EventFilters = { kind: null, target: null },
  pageSize = 10,
) {
  const client = { events: vi.fn(events) };
  render(EventsPage, {
    filters,
    client,
    overview: fixtures['overview.json'],
    visibility: visible,
    serverNow: NOW,
    pageSize,
  });
  await vi.advanceTimersByTimeAsync(0);
  return client;
}

function rows(): HTMLElement[] {
  return within(screen.getByRole('list', { name: 'Events' })).getAllByRole('listitem');
}

describe('EventsPage', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    location.hash = '#/events';
  });
  afterEach(() => vi.useRealTimers());

  it('renders the contract example, newest first, with links to targets', async () => {
    await renderPage(backendOver(fixtures['events.json'].events));
    const items = rows();
    expect(items).toHaveLength(4);
    expect(items[0]?.textContent).toContain('minecraft.service');
    expect(within(items[0]!).getByRole('link').getAttribute('href')).toBe(
      '#/services/minecraft.service',
    );
    expect(within(items[2]!).getByRole('link').getAttribute('href')).toBe('#/system');
    expect(within(items[3]!).getByRole('link').getAttribute('href')).toBe('#/library');
    // Only an empty page ends the log, so one more request settles it.
    await fireEvent.click(screen.getByRole('button', { name: 'Load older events' }));
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.getByText('Start of the log')).toBeTruthy();
  });

  it('loads older pages with the until cursor until an empty page ends the log', async () => {
    const all = log(25);
    const client = await renderPage(backendOver(all));
    expect(rows()).toHaveLength(10);
    expect(client.events).toHaveBeenLastCalledWith(
      { limit: 10, target: undefined, kind: undefined },
      expect.anything(),
    );

    const older = () => screen.getByRole('button', { name: 'Load older events' });
    await fireEvent.click(older());
    await vi.advanceTimersByTimeAsync(0);
    expect(client.events).toHaveBeenLastCalledWith(
      { until: all[9]!.ts, limit: 10, target: undefined, kind: undefined },
      expect.anything(),
    );
    expect(rows()).toHaveLength(20);

    // A short page is not the end: only the empty page after it is.
    await fireEvent.click(older());
    await vi.advanceTimersByTimeAsync(0);
    expect(rows()).toHaveLength(25);
    await fireEvent.click(older());
    await vi.advanceTimersByTimeAsync(0);
    expect(rows().map((r) => r.querySelector('.message')?.textContent)).toEqual(
      all.map((e) => e.message),
    );
    expect(screen.queryByRole('button', { name: 'Load older events' })).toBeNull();
    expect(screen.getByText('Start of the log')).toBeTruthy();
  });

  it('shows a page longer than the limit when the backend completes a second', async () => {
    const sameSecond = log(14).map((e, i) => ({ ...e, ts: i < 12 ? NOW : NOW - 60 }));
    await renderPage(backendOver(sameSecond));
    expect(rows()).toHaveLength(12);
  });

  it('passes the target filter to the API and hides the repeated target', async () => {
    const client = await renderPage(backendOver(log(6)), {
      kind: null,
      target: 'minecraft.service',
    });
    expect(client.events).toHaveBeenCalledWith(
      { limit: 10, target: 'minecraft.service', kind: undefined },
      expect.anything(),
    );
    expect(rows()).toHaveLength(3);
    expect(rows()[0]?.textContent).not.toContain('minecraft.service');
    expect(screen.getByRole('link', { name: 'Clear filters' }).getAttribute('href')).toBe(
      '#/events',
    );
  });

  it('passes the kind filter to the API and offers only that kind of target', async () => {
    const client = await renderPage(backendOver(log(6)), { kind: 'timer', target: null });
    expect(client.events).toHaveBeenCalledWith(
      { limit: 10, target: undefined, kind: 'timer' },
      expect.anything(),
    );
    expect(rows()).toHaveLength(3);
    expect(rows().every((r) => r.textContent?.includes('music-sync.timer'))).toBe(true);
    const target = screen.getByRole('combobox', { name: 'Target' });
    const offered = within(target)
      .getAllByRole('option')
      .map((o) => o.textContent);
    expect(offered).toEqual(['All targets', 'backup.timer', 'music-sync.timer']);
  });

  it('filters by the library kind', async () => {
    const client = await renderPage(backendOver(fixtures['events.json'].events), {
      kind: 'library',
      target: null,
    });
    expect(client.events).toHaveBeenCalledWith(
      { limit: 10, target: undefined, kind: 'library' },
      expect.anything(),
    );
    const kinds = within(screen.getByRole('combobox', { name: 'Kind' }))
      .getAllByRole('option')
      .map((o) => o.textContent);
    expect(kinds).toContain('Library');
    expect(rows()).toHaveLength(1);
    expect(within(rows()[0]!).getByRole('link').getAttribute('href')).toBe('#/library');
    const offered = within(screen.getByRole('combobox', { name: 'Target' }))
      .getAllByRole('option')
      .map((o) => o.textContent);
    expect(offered).toEqual(['All targets', 'library']);
  });

  it('puts filter changes in the URL, dropping a target of another kind', async () => {
    await renderPage(backendOver(log(6)), { kind: null, target: 'minecraft.service' });
    await fireEvent.change(screen.getByRole('combobox', { name: 'Kind' }), {
      target: { value: 'timer' },
    });
    expect(location.hash).toBe('#/events?kind=timer');

    await fireEvent.change(screen.getByRole('combobox', { name: 'Target' }), {
      target: { value: 'music-sync.timer' },
    });
    expect(location.hash).toBe('#/events?target=music-sync.timer');
  });

  it('says when a kind has no events', async () => {
    await renderPage(backendOver(log(6)), { kind: 'check', target: null });
    expect(screen.getByText('No check events yet')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Load older events' })).toBeNull();
  });

  it('has empty, loading and error states', async () => {
    await renderPage(async () => ({ host: 'debian-box', events: [] }), {
      kind: null,
      target: 'quiet.service',
    });
    expect(screen.getByText('No events for quiet.service')).toBeTruthy();
  });

  it('shows loading, then an error with retry', async () => {
    const client = await renderPage(async () => {
      throw new ApiRequestError('network', 'Failed to fetch', 0);
    });
    expect(screen.getByText('Could not load events')).toBeTruthy();
    client.events.mockResolvedValue(fixtures['events.json']);
    await fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    await vi.advanceTimersByTimeAsync(0);
    expect(rows()).toHaveLength(4);
  });

  it('reports a failed older page without losing what is shown', async () => {
    const backend = backendOver(log(25));
    const client = await renderPage(backend);
    client.events.mockRejectedValueOnce(new ApiRequestError('network', 'Failed to fetch', 0));
    await fireEvent.click(screen.getByRole('button', { name: 'Load older events' }));
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.getByRole('alert').textContent).toContain('Could not load older events');
    expect(rows()).toHaveLength(10);
  });

  it('picks up new events when polling', async () => {
    const all = log(5);
    const backend = backendOver(all);
    await renderPage(backend);
    all.unshift({ ...all[0]!, id: 2000, ts: NOW + 30, message: 'fresh' });
    await vi.advanceTimersByTimeAsync(30_000);
    expect(rows()[0]?.textContent).toContain('fresh');
    expect(rows()).toHaveLength(6);
  });
});
