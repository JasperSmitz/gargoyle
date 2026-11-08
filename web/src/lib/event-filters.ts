// The events page filters, kept in the URL (`#/events?kind=timer&target=backup.timer`) so a
// filtered view can be linked and survives a reload.

import { TARGET_KINDS, type HealthEvent, type Overview, type TargetKind } from './api/types';

export interface EventFilters {
  kind: TargetKind | null;
  target: string | null;
}

export function parseEventFilters(query: URLSearchParams): EventFilters {
  const kind = query.get('kind');
  const target = query.get('target');
  return {
    kind: TARGET_KINDS.find((k) => k === kind) ?? null,
    target: target === null || target === '' ? null : target,
  };
}

export interface TargetOption {
  kind: TargetKind;
  target: string;
}

/**
 * Targets offered by the target filter: every configured unit and check, plus any target seen in
 * the loaded events (system targets such as `disk:/` are only known that way). Sorted by kind
 * order, then name.
 */
export function targetOptions(
  overview: Overview | null,
  events: readonly HealthEvent[],
): TargetOption[] {
  const options = new Map<string, TargetOption>();
  const add = (kind: TargetKind, target: string) => {
    if (!options.has(target)) options.set(target, { kind, target });
  };
  for (const s of overview?.services ?? []) add('service', s.unit);
  for (const t of overview?.timers ?? []) add('timer', t.unit);
  for (const c of overview?.checks ?? []) add('check', c.name);
  if (overview?.library) add('library', 'library');
  for (const e of events) add(e.targetKind, e.target);
  const order = (kind: TargetKind) => TARGET_KINDS.indexOf(kind);
  return [...options.values()].sort(
    (a, b) => order(a.kind) - order(b.kind) || a.target.localeCompare(b.target),
  );
}
