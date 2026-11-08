// The canonical contract examples (docs/api/examples) as typed fixtures. Each `satisfies` makes
// `pnpm check` fail when the TS types and the examples drift apart; the cast afterwards only
// narrows widened strings back to their literal unions, whose values `contract.test.ts` checks.
// Also usable as offline data for UI work.

import type { Conforms } from './contract';
import type {
  ApiError,
  CheckDetail,
  ChecksResponse,
  EventsResponse,
  HealthResponse,
  LibraryResponse,
  MetricsQuery,
  MetricsResponse,
  Overview,
  ServiceDetail,
  ServicesResponse,
  TimerDetail,
  TimersResponse,
} from './types';

import checkDetailJson from '../../../../docs/api/examples/check-detail.json';
import checksJson from '../../../../docs/api/examples/checks.json';
import errorJson from '../../../../docs/api/examples/error.json';
import eventsJson from '../../../../docs/api/examples/events.json';
import healthJson from '../../../../docs/api/examples/health.json';
import libraryUnconfiguredJson from '../../../../docs/api/examples/library-unconfigured.json';
import libraryJson from '../../../../docs/api/examples/library.json';
import metricsQueryJson from '../../../../docs/api/examples/metrics-query.json';
import metricsJson from '../../../../docs/api/examples/metrics.json';
import overviewJson from '../../../../docs/api/examples/overview.json';
import serviceDetailJson from '../../../../docs/api/examples/service-detail.json';
import servicesJson from '../../../../docs/api/examples/services.json';
import timerDetailJson from '../../../../docs/api/examples/timer-detail.json';
import timersJson from '../../../../docs/api/examples/timers.json';

/** Keyed by example file name, so tests can check every file in the directory is covered. */
export const fixtures = {
  'check-detail.json': checkDetailJson satisfies Conforms<
    typeof checkDetailJson,
    CheckDetail
  > as CheckDetail,
  'checks.json': checksJson satisfies Conforms<typeof checksJson, ChecksResponse> as ChecksResponse,
  'error.json': errorJson satisfies Conforms<typeof errorJson, ApiError> as ApiError,
  'events.json': eventsJson satisfies Conforms<typeof eventsJson, EventsResponse> as EventsResponse,
  'health.json': healthJson satisfies Conforms<typeof healthJson, HealthResponse> as HealthResponse,
  'library.json': libraryJson satisfies Conforms<
    typeof libraryJson,
    LibraryResponse
  > as LibraryResponse,
  'library-unconfigured.json': libraryUnconfiguredJson satisfies Conforms<
    typeof libraryUnconfiguredJson,
    LibraryResponse
  > as LibraryResponse,
  'metrics-query.json': metricsQueryJson satisfies Conforms<
    typeof metricsQueryJson,
    MetricsQuery
  > as MetricsQuery,
  'metrics.json': metricsJson satisfies Conforms<
    typeof metricsJson,
    MetricsResponse
  > as MetricsResponse,
  'overview.json': overviewJson satisfies Conforms<typeof overviewJson, Overview> as Overview,
  'service-detail.json': serviceDetailJson satisfies Conforms<
    typeof serviceDetailJson,
    ServiceDetail
  > as ServiceDetail,
  'services.json': servicesJson satisfies Conforms<
    typeof servicesJson,
    ServicesResponse
  > as ServicesResponse,
  'timer-detail.json': timerDetailJson satisfies Conforms<
    typeof timerDetailJson,
    TimerDetail
  > as TimerDetail,
  'timers.json': timersJson satisfies Conforms<typeof timersJson, TimersResponse> as TimersResponse,
};
