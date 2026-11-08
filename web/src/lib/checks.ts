// Presentation helpers shared by the checks table and the check detail page.

import type { CheckKind, CheckSummary } from './api/types';
import type { ChartGroup } from './charts/groups';
import { formatValue } from './charts/units';
import { singleSeries } from './service-charts';

export const CHECK_KIND_LABEL: Record<CheckKind, string> = { http: 'HTTP', tcp: 'TCP' };

/** "3.1 ms", "1.20 s"; null when the last probe failed or none has run. */
export function formatLatency(check: CheckSummary): string | null {
  return check.latencyMs === null ? null : formatValue('ms', check.latencyMs);
}

/** Metric key of a check's latency series (ARCHITECTURE §5). */
export function latencyKey(name: string): string {
  return `check.latency:${name}`;
}

/** The check page's one chart: probe latency in ms (only answered probes have a latency). */
export function checkChartGroups(name: string): ChartGroup[] {
  return [singleSeries('check-latency', latencyKey(name), 'Latency', 'ms', 'Latency')];
}
