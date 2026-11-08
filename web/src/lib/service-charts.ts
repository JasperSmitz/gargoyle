// The charts on a service's page: its memory and restart-count series (ARCHITECTURE §5,
// `service.memory:<unit>` in bytes, `service.restarts:<unit>` as a count). One query each, so
// a missing series affects only its own chart.

import { ApiRequestError, type ApiClient } from './api/client';
import type { MetricsQuery, SeriesUnit } from './api/types';
import type { ChartGroup } from './charts/groups';

/** A group of one chart over one exact series key. */
export function singleSeries(
  id: string,
  key: string,
  title: string,
  unit: SeriesUnit,
  label: string,
) {
  return {
    id,
    queryKeys: [key],
    charts: [{ id, title, unit, series: [{ key, label, slot: 0 }] }],
  } satisfies ChartGroup;
}

export function serviceChartGroups(unit: string): ChartGroup[] {
  return [
    singleSeries('service-memory', `service.memory:${unit}`, 'Memory use', 'bytes', 'Memory'),
    singleSeries('service-restarts', `service.restarts:${unit}`, 'Restarts', 'count', 'Restarts'),
  ];
}

/**
 * A series is created by its first sample, and the backend answers an exact unknown key with
 * 400 `unknown_series` (§5): a service that never reported memory, or a check that never
 * answered, has no series yet. For these charts that means "no data", not a failure. The series
 * list is consulted first so no failing request is made (the browser would log it as an error);
 * keys are remembered once listed. The 400 is still treated as empty in case a series disappears
 * between the two requests.
 */
export function treatUnknownKeyAsEmpty(
  client: Pick<ApiClient, 'metricsQuery' | 'metrics'>,
): Pick<ApiClient, 'metricsQuery'> {
  const listed = new Set<string>();
  return {
    metricsQuery: async (query, req) => {
      const exact = query.keys.filter((key) => !key.includes('*'));
      if (exact.some((key) => !listed.has(key))) {
        const { series } = await client.metrics(req);
        for (const info of series) listed.add(info.key);
        const allUnknown = exact.length === query.keys.length && exact.every((k) => !listed.has(k));
        if (allUnknown) return emptyResult(query);
      }
      try {
        return await client.metricsQuery(query, req);
      } catch (error) {
        if (!isUnknownKey(error)) throw error;
        return emptyResult(query);
      }
    },
  };
}

function emptyResult(query: { from?: number; to?: number }): MetricsQuery {
  return {
    host: '',
    from: query.from ?? 0,
    to: query.to ?? 0,
    step: 0,
    resolution: 'raw',
    series: [],
  };
}

/** Error code of a 400 for a series key the backend has never stored. */
export const UNKNOWN_SERIES = 'unknown_series';

function isUnknownKey(error: unknown): boolean {
  return error instanceof ApiRequestError && error.code === UNKNOWN_SERIES;
}
