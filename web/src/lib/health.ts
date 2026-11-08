// How health states are presented. Colors live in tokens.css (`--health-<state>-*`); every state
// also has a distinct icon shape and a text label so color is never the only signal.

import { HEALTH_STATES, type Health } from './api/types';

export interface HealthPresentation {
  label: string;
  /** Distinct silhouettes, readable without color: circle-check, triangle, octagon, circle-? */
  icon: 'check' | 'alert' | 'octagon' | 'question';
}

export const HEALTH_PRESENTATION: Record<Health, HealthPresentation> = {
  healthy: { label: 'Healthy', icon: 'check' },
  warning: { label: 'Warning', icon: 'alert' },
  critical: { label: 'Critical', icon: 'octagon' },
  unknown: { label: 'Unknown', icon: 'question' },
};

/** ARCHITECTURE §8 ordering: critical > warning > unknown > healthy. Higher is worse. */
const SEVERITY: Record<Health, number> = { healthy: 0, unknown: 1, warning: 2, critical: 3 };

export function severity(health: Health): number {
  return SEVERITY[health];
}

/** Worst state of `states`; `healthy` for none, matching how the backend aggregates. */
export function worst(states: Iterable<Health>): Health {
  let result: Health = 'healthy';
  for (const state of states) if (SEVERITY[state] > SEVERITY[result]) result = state;
  return result;
}

/** Anything that is not healthy needs a look, including unknown (stale or not yet run). */
export function needsAttention(health: Health): boolean {
  return health !== 'healthy';
}

/** Worst first; items of equal health keep their configured order (the sort is stable). */
export function sortBySeverity<T extends { health: Health }>(items: readonly T[]): T[] {
  return [...items].sort((a, b) => SEVERITY[b.health] - SEVERITY[a.health]);
}

/** Health states from worst to best, for summaries such as "2 critical · 1 warning". */
export const HEALTH_BY_SEVERITY: readonly Health[] = [...HEALTH_STATES].sort(
  (a, b) => SEVERITY[b] - SEVERITY[a],
);
