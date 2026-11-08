// @vitest-environment jsdom
import { render, screen, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import { fixtures } from '../../lib/api/fixtures';
import type { TimerSummary } from '../../lib/api/types';
import TimerTable from './TimerTable.svelte';

const NOW = 1_794_200_000;
const [failed, never] = fixtures['timers.json'].timers as [TimerSummary, TimerSummary];

function rows(timers: TimerSummary[], now = NOW) {
  render(TimerTable, { timers, serverNow: now });
  return screen.getAllByRole('row').slice(1);
}

function cell(row: HTMLElement | undefined, column: number): string {
  return (row?.querySelectorAll('th, td')[column]?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

describe('TimerTable', () => {
  it('has the four columns', () => {
    rows([failed]);
    const headers = screen.getAllByRole('columnheader').map((h) => h.textContent?.trim());
    expect(headers).toEqual(['Timer', 'Last run', 'Next run', 'Health']);
  });

  it('shows last run as relative time plus result, and next run as a countdown', () => {
    const [row] = rows([failed]);
    expect(cell(row, 1)).toBe('2h ago exit-code (status 1)');
    expect(cell(row, 2)).toBe('in 21h 13m');
    expect(cell(row, 3)).toBe('Critical');
    expect(within(row!).getByRole('link', { name: 'Music sync' }).getAttribute('href')).toBe(
      '#/timers/music-sync.timer',
    );
  });

  it('says a timer never ran, with the message under the name', () => {
    const [row] = rows([never]);
    expect(cell(row, 1)).toBe('Never run');
    expect(row?.querySelector('.message')?.textContent).toBe('never run');
  });

  it('shows the message of a failed timer under its name', () => {
    const [row] = rows([failed]);
    expect(row?.querySelector('.message')?.textContent).toBe(failed.message);
  });

  it('shows a running timer and when it started', () => {
    const running: TimerSummary = {
      ...failed,
      health: 'healthy',
      message: null,
      running: true,
      lastRun: {
        startedAt: NOW - 300,
        finishedAt: null,
        duration: null,
        result: null,
        exitStatus: null,
      },
    };
    const [row] = rows([running]);
    expect(cell(row, 1)).toBe('Running now started 5m ago');
  });

  it('does not date a new run by the previous one when only the flag says running', () => {
    const [row] = rows([{ ...failed, health: 'healthy', message: null, running: true }]);
    expect(cell(row, 1)).toBe('Running now');
  });

  it('explains warnings the columns do not show, and marks inactive timers', () => {
    const stopped: TimerSummary = {
      ...failed,
      health: 'warning',
      message: 'timer is not active',
      active: false,
      nextRun: null,
      lastRun: { ...failed.lastRun!, result: 'success', exitStatus: 0 },
    };
    const [row] = rows([stopped]);
    expect(cell(row, 0)).toContain('timer is not active');
    expect(cell(row, 2)).toBe('Not scheduled timer inactive');
  });

  it('marks an overdue next run', () => {
    const [row] = rows([{ ...failed, nextRun: NOW - 900 }]);
    expect(cell(row, 2)).toBe('overdue 15m');
  });

  it('lists the worst timers first', () => {
    const healthy = { ...never, unit: 'ok.timer', name: 'OK', health: 'healthy' as const };
    const names = rows([healthy, never, failed]).map((r) => cell(r, 0));
    expect(names[0]).toContain('Music sync');
    expect(names.at(-1)).toContain('OK');
  });
});
