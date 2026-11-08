// The "needs attention" list at the top of the overview: every non-healthy target, worst first.
// Services, timers and checks carry a backend `message`; system metrics only carry a health, so
// their line is built from the current value instead.

import type { Health, Overview, TargetKind } from './api/types';
import { formatValue } from './charts/units';
import { formatLoad, formatPercent, isTimestamp } from './format';
import { HEALTH_PRESENTATION, needsAttention, sortBySeverity } from './health';
import { targetHref } from './router';

export interface Problem {
  kind: TargetKind;
  /** Stable within one overview; used as the list key. */
  id: string;
  name: string;
  health: Health;
  message: string;
  /** The page with more detail. */
  href: string;
}

export function collectProblems(overview: Overview): Problem[] {
  const { system, hostInfo } = overview;
  const problems: Problem[] = [];

  const add = (kind: TargetKind, id: string, name: string, health: Health, message: string) => {
    if (!needsAttention(health)) return;
    problems.push({ kind, id: `${kind}:${id}`, name, health, message, href: targetHref(kind, id) });
  };

  if (!isTimestamp(system.ts)) {
    // Before the first sample the backend sends `ts = 0` and placeholder values; one line suffices.
    add('system', 'system', 'System', system.health, 'waiting for the first sample');
  } else {
    add('system', 'cpu', 'CPU', system.cpu.health, `${formatPercent(system.cpu.usage)} busy`);
    add(
      'system',
      'load',
      'Load',
      system.load.health,
      `${formatLoad(system.load.one)} (1 min) on ${hostInfo.cores} cores`,
    );
    add(
      'system',
      'memory',
      'Memory',
      system.memory.health,
      `${formatPercent(system.memory.usedPct)} used`,
    );
    for (const disk of system.disks) {
      add(
        'system',
        `disk:${disk.mount}`,
        `Disk ${disk.mount}`,
        disk.health,
        `${formatPercent(disk.usedPct)} used`,
      );
    }
    for (const temp of system.temps) {
      add(
        'system',
        `temp:${temp.label}`,
        temp.label,
        temp.health,
        formatValue('celsius', temp.celsius),
      );
    }
    // System health can be worse than any part, e.g. `unknown` when the collector is stale.
    if (needsAttention(system.health) && !problems.some((p) => p.health === system.health)) {
      add('system', 'system', 'System', system.health, HEALTH_PRESENTATION[system.health].label);
    }
  }

  const fallback = (health: Health, message: string | null) =>
    message ?? HEALTH_PRESENTATION[health].label;
  for (const s of overview.services) {
    add('service', s.unit, s.name, s.health, fallback(s.health, s.message));
  }
  for (const t of overview.timers) {
    add('timer', t.unit, t.name, t.health, fallback(t.health, t.message));
  }
  for (const c of overview.checks) {
    add('check', c.name, c.name, c.health, fallback(c.health, c.message));
  }

  return sortBySeverity(problems);
}
