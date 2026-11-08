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
  TargetKind,
  ServiceDetail,
  ServicesResponse,
  TimerDetail,
  TimersResponse,
  UnixSeconds,
} from './types';

/**
 * Every failed request rejects with this. `code` is the server's `ApiError` code when the body
 * has that shape; otherwise one of the client-side codes below. `status` is 0 when no HTTP
 * response was received.
 */
export class ApiRequestError extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly status: number,
  ) {
    super(message);
    this.name = 'ApiRequestError';
  }
}

/** Client-side error codes, used when the server did not supply an `ApiError`. */
export const CLIENT_ERROR_CODES = {
  network: 'network',
  http: 'http_error',
  invalidResponse: 'invalid_response',
} as const;

export function isApiError(value: unknown): value is ApiError {
  if (typeof value !== 'object' || value === null || !('error' in value)) return false;
  const error = value.error;
  return (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    typeof error.code === 'string' &&
    'message' in error &&
    typeof error.message === 'string'
  );
}

type Query = Record<string, string | number | readonly string[] | null | undefined>;

/** Per-request options accepted by every client method. */
export interface RequestOptions {
  /**
   * Aborting rejects with the signal's `reason` (an `AbortError` `DOMException` by default),
   * exactly like `fetch`, never with an `ApiRequestError`: an abort is the caller's decision,
   * not a failure. Use `isAbortError` to tell the two apart.
   */
  signal?: AbortSignal;
}

/** True for the rejection produced by aborting a request through `RequestOptions.signal`. */
export function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === 'AbortError';
}

export interface ClientOptions {
  /** Defaults to the global `fetch`; injectable for tests. */
  fetch?: typeof fetch;
  baseUrl?: string;
}

export function createClient({ fetch: fetchImpl, baseUrl = '/api/v1' }: ClientOptions = {}) {
  // Resolve the global lazily so it is called unbound and can be stubbed after creation.
  const doFetch: typeof fetch = fetchImpl ?? ((input, init) => fetch(input, init));

  async function get<T>(
    path: string,
    query: Query = {},
    { signal }: RequestOptions = {},
  ): Promise<T> {
    const url = baseUrl + path + queryString(query);

    let response: Response;
    try {
      response = await doFetch(url, { headers: { Accept: 'application/json' }, signal });
    } catch (cause) {
      rethrowIfAborted(signal);
      throw new ApiRequestError(CLIENT_ERROR_CODES.network, describe(cause), 0);
    }

    let body: unknown;
    try {
      body = await response.json();
    } catch {
      // Aborting while the body streams also lands here; that must not read as a bad body.
      rethrowIfAborted(signal);
      body = undefined;
    }

    if (!response.ok) {
      if (isApiError(body)) {
        throw new ApiRequestError(body.error.code, body.error.message, response.status);
      }
      const reason = response.statusText ? ` ${response.statusText}` : '';
      throw new ApiRequestError(
        CLIENT_ERROR_CODES.http,
        `HTTP ${response.status}${reason}`,
        response.status,
      );
    }
    // Every contract response is a JSON object; anything else is not from a Gargoyle backend.
    if (typeof body !== 'object' || body === null || Array.isArray(body)) {
      throw new ApiRequestError(
        CLIENT_ERROR_CODES.invalidResponse,
        body === undefined ? 'response is not valid JSON' : 'response is not a JSON object',
        response.status,
      );
    }
    // The contract is trusted rather than validated at runtime; see fixtures.ts for drift checks.
    return body as T;
  }

  return {
    health: (req?: RequestOptions) => get<HealthResponse>('/health', {}, req),
    overview: (req?: RequestOptions) => get<Overview>('/overview', {}, req),
    services: (req?: RequestOptions) => get<ServicesResponse>('/services', {}, req),
    service: (unit: string, req?: RequestOptions) =>
      get<ServiceDetail>(`/services/${encodeURIComponent(unit)}`, {}, req),
    timers: (req?: RequestOptions) => get<TimersResponse>('/timers', {}, req),
    timer: (unit: string, opts: { limit?: number } = {}, req?: RequestOptions) =>
      get<TimerDetail>(`/timers/${encodeURIComponent(unit)}`, opts, req),
    checks: (req?: RequestOptions) => get<ChecksResponse>('/checks', {}, req),
    check: (name: string, req?: RequestOptions) =>
      get<CheckDetail>(`/checks/${encodeURIComponent(name)}`, {}, req),
    metrics: (req?: RequestOptions) => get<MetricsResponse>('/metrics', {}, req),
    metricsQuery: (
      opts: {
        keys: readonly string[];
        from?: UnixSeconds;
        to?: UnixSeconds;
        points?: number;
      },
      req?: RequestOptions,
    ) =>
      get<MetricsQuery>(
        '/metrics/query',
        { key: opts.keys, from: opts.from, to: opts.to, points: opts.points },
        req,
      ),
    library: (req?: RequestOptions) => get<LibraryResponse>('/library', {}, req),
    events: (
      opts: {
        since?: UnixSeconds;
        /** Exclusive. */
        until?: UnixSeconds;
        target?: string;
        kind?: TargetKind;
        limit?: number;
      } = {},
      req?: RequestOptions,
    ) => get<EventsResponse>('/events', opts, req),
  };
}

export type ApiClient = ReturnType<typeof createClient>;

export const api: ApiClient = createClient();

function queryString(query: Query): string {
  const params = new URLSearchParams();
  for (const [name, value] of Object.entries(query)) {
    if (value === null || value === undefined) continue;
    if (typeof value === 'object') {
      for (const item of value) params.append(name, item);
    } else {
      params.append(name, String(value));
    }
  }
  const encoded = params.toString();
  return encoded ? `?${encoded}` : '';
}

function rethrowIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted) throw signal.reason;
}

function describe(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}
