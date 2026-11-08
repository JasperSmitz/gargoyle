import { describe, expect, it } from 'vitest';
import type { TimerRunSummary } from './api/types';
import {
  MIN_BAR_FRACTION,
  durationFractions,
  isRunning,
  outcomeLabel,
  runDuration,
  runOutcome,
} from './timers';

const finished = (result: string, exitStatus: number, duration = 42): TimerRunSummary => ({
  startedAt: 1000,
  finishedAt: 1000 + duration,
  duration,
  result,
  exitStatus,
});
const running: TimerRunSummary = {
  startedAt: 1000,
  finishedAt: null,
  duration: null,
  result: null,
  exitStatus: null,
};

describe('run outcome', () => {
  it.each([
    [finished('success', 0), 'success', 'Succeeded'],
    [finished('exit-code', 1), 'failed', 'exit-code (status 1)'],
    [finished('timeout', 0), 'failed', 'timeout'],
    [finished('signal', 9), 'failed', 'signal (status 9)'],
    [running, 'running', 'Running'],
    // systemd sets Result=success at start; an unfinished run is running whatever it says.
    [{ ...running, result: 'success', exitStatus: 0 }, 'running', 'Running'],
  ])('%j is %s, %s', (run, outcome, label) => {
    expect(runOutcome(run)).toBe(outcome);
    expect(outcomeLabel(run)).toBe(label);
  });
});

describe('runDuration', () => {
  it('uses the reported duration', () => {
    expect(runDuration(finished('success', 0, 38), 99_999)).toBe(38);
  });

  it('falls back to finish minus start', () => {
    expect(runDuration({ ...finished('success', 0), duration: null }, 99_999)).toBe(42);
  });

  it('counts a run with a premature result up to now', () => {
    expect(runDuration({ ...running, result: 'success' }, 1300)).toBe(300);
  });

  it('counts a running run up to now', () => {
    expect(runDuration(running, 1300)).toBe(300);
    expect(runDuration(running, 900)).toBe(0); // client clock behind the server's
  });
});

describe('durationFractions', () => {
  it('scales to the longest run', () => {
    expect(durationFractions([42, 21, 84])).toEqual([0.5, 0.25, 1]);
  });

  it('keeps tiny runs visible and skips unknown ones', () => {
    expect(durationFractions([1000, 0, null])).toEqual([1, MIN_BAR_FRACTION, null]);
  });

  it('copes with nothing to compare', () => {
    expect(durationFractions([])).toEqual([]);
    expect(durationFractions([0, 0])).toEqual([MIN_BAR_FRACTION, MIN_BAR_FRACTION]);
  });
});

describe('isRunning', () => {
  const timer = {
    unit: 'a.timer',
    name: 'a',
    service: 'a.service',
    health: 'healthy' as const,
    message: null,
    active: true,
    nextRun: null,
  };
  it.each([
    [false, finished('success', 0), false],
    [true, finished('success', 0), true],
    [false, { ...running, result: 'success', exitStatus: 0 }, true],
    [false, null, false],
  ])('running flag %s with last run %j -> %s', (flag, lastRun, expected) => {
    expect(isRunning({ ...timer, running: flag, lastRun })).toBe(expected);
  });
});
