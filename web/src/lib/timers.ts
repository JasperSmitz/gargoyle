// Timer run presentation shared by the overview's timers table and the timer detail page.
// A run's outcome is good or bad, so it is shown with health colors and icons (never color alone).

import type { Health, TimerRunSummary, TimerSummary } from './api/types';

export type RunOutcome = 'success' | 'failed' | 'running';

/**
 * A run that has not finished is in progress (ARCHITECTURE §6, nullable `TimerRun` fields).
 * `finishedAt` decides, not `result`: systemd sets `Result=success` when a run starts, so a
 * collector copying it verbatim would otherwise make a running run read as succeeded.
 */
export function runOutcome(run: TimerRunSummary): RunOutcome {
  if (run.finishedAt === null || run.result === null) return 'running';
  return run.result === 'success' ? 'success' : 'failed';
}

/**
 * Whether the timer's service is running now, the one test every view uses. The backend's flag
 * and an unfinished last run both count, so the views cannot contradict each other.
 */
export function isRunning(timer: TimerSummary): boolean {
  return timer.running || (timer.lastRun !== null && runOutcome(timer.lastRun) === 'running');
}

/** The health icon that goes with an outcome; a running run is not a verdict yet. */
export const OUTCOME_HEALTH: Record<RunOutcome, Health> = {
  success: 'healthy',
  failed: 'critical',
  running: 'unknown',
};

/** `Succeeded`, `Running`, or the systemd result with a non-zero exit status: `exit-code (status 1)`. */
export function outcomeLabel(run: TimerRunSummary): string {
  switch (runOutcome(run)) {
    case 'running':
      return 'Running';
    case 'success':
      return 'Succeeded';
    case 'failed': {
      const status =
        run.exitStatus !== null && run.exitStatus !== 0 ? ` (status ${run.exitStatus})` : '';
      return `${run.result}${status}`;
    }
  }
}

/** Seconds a run took, or has been running for so far; null when unknowable. */
export function runDuration(run: TimerRunSummary, now: number): number | null {
  if (runOutcome(run) === 'running') return Math.max(0, now - run.startedAt);
  if (run.duration !== null) return run.duration;
  return run.finishedAt !== null ? Math.max(0, run.finishedAt - run.startedAt) : null;
}

/** Bars shorter than this would vanish; a 0s run still deserves a visible stub. */
export const MIN_BAR_FRACTION = 0.01;

/**
 * Bar lengths for run durations as fractions of the longest one shown, so the history reads as a
 * comparison of runs. Null durations get no bar. All-zero durations get stubs.
 */
export function durationFractions(durations: readonly (number | null)[]): (number | null)[] {
  const longest = Math.max(0, ...durations.map((d) => d ?? 0));
  return durations.map((d) => {
    if (d === null) return null;
    if (longest === 0) return MIN_BAR_FRACTION;
    return Math.max(MIN_BAR_FRACTION, d / longest);
  });
}
