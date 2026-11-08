// @vitest-environment jsdom
import { render, screen, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiRequestError } from '../lib/api/client';
import { fixtures } from '../lib/api/fixtures';
import type { TimerDetail } from '../lib/api/types';
import type { Visibility } from '../lib/poller';
import TimerPage from './TimerPage.svelte';

const NOW = 1_794_200_000;
const visible: Visibility = { isHidden: () => false, subscribe: () => () => {} };
const example = fixtures['timer-detail.json'];

async function renderPage(load: () => Promise<TimerDetail>, unit = 'music-sync.timer') {
  const client = { timer: vi.fn(load) };
  render(TimerPage, { unit, client, visibility: visible, serverNow: NOW });
  await vi.advanceTimersByTimeAsync(0);
  return client;
}

function region(name: string | RegExp): HTMLElement {
  return screen.getByRole('region', { name });
}

describe('TimerPage', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('requests the unit with a run limit', async () => {
    const client = await renderPage(async () => example);
    expect(client.timer).toHaveBeenCalledWith('music-sync.timer', { limit: 50 }, expect.anything());
  });

  it('summarises the timer: health, reason, last and next run', async () => {
    await renderPage(async () => example);
    const summary = region('Timer summary');
    expect(within(summary).getByRole('heading', { level: 1 }).textContent).toBe('Music sync');
    expect(summary.textContent).toContain('music-sync.timer');
    expect(summary.textContent).toContain('last run failed: exit-code (status 1)');
    expect(summary.querySelector('[data-health="critical"]')).not.toBeNull();
    const facts = summary.querySelector('dl')?.textContent?.replace(/\s+/g, ' ');
    expect(facts).toContain('Last run 2h ago exit-code (status 1)');
    expect(facts).toContain('Took 42s');
    expect(facts).toContain('Next run in 21h 13m');
    expect(facts).toContain('music-sync.service');
  });

  it('draws run history with duration bars scaled to the longest run and the result', async () => {
    await renderPage(async () => example);
    const runs = within(region(/^Run history/)).getAllByRole('listitem');
    expect(runs).toHaveLength(2);
    expect(runs[0]?.textContent).toContain('exit-code (status 1)');
    expect(runs[0]?.textContent).toContain('42s');
    expect(runs[1]?.textContent).toContain('Succeeded');
    expect(runs[1]?.textContent).toContain('journal');
    const bars = runs.map((r) => r.querySelector<HTMLElement>('.fill'));
    expect(bars[0]?.classList.contains('failed')).toBe(true);
    expect(bars[0]?.style.getPropertyValue('--fraction')).toBe('1');
    expect(Number(bars[1]?.style.getPropertyValue('--fraction'))).toBeCloseTo(38 / 42);
    expect(region(/^Run history/).textContent).toContain('the longest shown took 42s');
  });

  it('shows a run in progress growing to now', async () => {
    const running = structuredClone(example);
    running.timer.running = true;
    running.runs.unshift({
      startedAt: NOW - 90,
      finishedAt: null,
      duration: null,
      result: null,
      exitStatus: null,
      source: 'observed',
    });
    await renderPage(async () => running);
    const [first] = within(region(/^Run history/)).getAllByRole('listitem');
    expect(first?.textContent).toContain('Running');
    expect(first?.textContent).toContain('1m 30s so far');
    expect(first?.querySelector('.fill.running')).not.toBeNull();
  });

  it('treats a run with a premature Result=success but no finish as running', async () => {
    const started = structuredClone(example);
    started.timer.running = true;
    started.timer.lastRun = {
      startedAt: NOW - 90,
      finishedAt: null,
      duration: null,
      result: 'success',
      exitStatus: 0,
    };
    await renderPage(async () => started);
    const facts = region('Timer summary').querySelector('dl')?.textContent?.replace(/\s+/g, ' ');
    expect(facts).toContain('Last run 1m ago Running');
    expect(facts).toContain('Running for 1m 30s');
    expect(facts).not.toContain('Succeeded');
  });

  it('lists the timer events without repeating the target', async () => {
    await renderPage(async () => example);
    const events = region(/^Events/);
    const [event] = within(events).getAllByRole('listitem');
    expect(event?.textContent).toMatch(/Healthy\s*→\s*Critical/);
    expect(event?.textContent).not.toContain('music-sync.timer');
    expect(within(events).getByRole('link').getAttribute('href')).toBe(
      '#/events?target=music-sync.timer',
    );
  });

  it('says when a timer has never run', async () => {
    const never = structuredClone(example);
    never.timer.lastRun = null;
    never.timer.health = 'unknown';
    never.timer.message = 'never run';
    never.runs = [];
    never.events = [];
    await renderPage(async () => never);
    expect(region('Timer summary').textContent).toContain('Timer has never run');
    const history = region(/^Run history/);
    expect(history.textContent).toContain('No runs recorded yet');
    expect(history.textContent).toContain('Timer has never run.');
    expect(region(/^Events/).textContent).toContain('No health changes recorded');
  });

  it('shows a loading state first', () => {
    render(TimerPage, {
      unit: 'x.timer',
      client: { timer: () => new Promise<TimerDetail>(() => {}) },
      visibility: visible,
      serverNow: NOW,
    });
    expect(screen.getByRole('status').textContent).toContain('Loading timer');
  });

  it('shows not found for an unknown unit', async () => {
    await renderPage(async () => {
      throw new ApiRequestError('not_found', 'unknown timer', 404);
    }, 'nope.timer');
    expect(screen.getByText('No timer called nope.timer')).toBeTruthy();
    expect(screen.queryByRole('region', { name: /Run history/ })).toBeNull();
  });

  it('offers a retry when loading fails, and keeps data when a refresh fails', async () => {
    const client = await renderPage(async () => {
      throw new ApiRequestError('network', 'Failed to fetch', 0);
    });
    expect(screen.getByText('Could not load this timer')).toBeTruthy();
    client.timer.mockResolvedValue(example);
    screen.getByRole('button', { name: 'Try again' }).click();
    await vi.advanceTimersByTimeAsync(0);
    expect(region('Timer summary')).toBeTruthy();

    client.timer.mockRejectedValue(new ApiRequestError('network', 'Failed to fetch', 0));
    await vi.advanceTimersByTimeAsync(30_000);
    expect(screen.getByRole('alert').textContent).toContain('Showing the last data received');
    expect(region('Timer summary')).toBeTruthy();
  });
});
