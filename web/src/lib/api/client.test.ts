import { describe, expect, it, vi } from 'vitest';
import { ApiRequestError, createClient, isAbortError, isApiError } from './client';
import { fixtures } from './fixtures';

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function clientReturning(response: Response | Error) {
  const fetch = vi.fn<typeof globalThis.fetch>(async () => {
    if (response instanceof Error) throw response;
    return response;
  });
  return { fetch, api: createClient({ fetch }) };
}

function requestedUrl(fetch: ReturnType<typeof clientReturning>['fetch']): string {
  return String(fetch.mock.calls[0]?.[0]);
}

async function rejection(promise: Promise<unknown>): Promise<ApiRequestError> {
  const error: unknown = await promise.then(
    () => {
      throw new Error('expected the request to fail');
    },
    (e: unknown) => e,
  );
  expect(error).toBeInstanceOf(ApiRequestError);
  return error as ApiRequestError;
}

describe('api client', () => {
  it('returns the parsed body on success', async () => {
    const { fetch, api } = clientReturning(jsonResponse(fixtures['health.json']));
    await expect(api.health()).resolves.toEqual({ status: 'ok', version: '0.1.0' });
    expect(requestedUrl(fetch)).toBe('/api/v1/health');
  });

  it('surfaces the server ApiError code, message and status', async () => {
    const { api } = clientReturning(jsonResponse(fixtures['error.json'], 404));
    const error = await rejection(api.service('foo.service'));
    expect(error).toMatchObject({
      code: 'not_found',
      message: 'unknown service: foo.service',
      status: 404,
    });
  });

  it('falls back to an http_error for non-JSON error bodies', async () => {
    const { api } = clientReturning(
      new Response('<html>Bad Gateway</html>', { status: 502, statusText: 'Bad Gateway' }),
    );
    const error = await rejection(api.overview());
    expect(error).toMatchObject({
      code: 'http_error',
      status: 502,
      message: 'HTTP 502 Bad Gateway',
    });
  });

  it('reports network failures with status 0', async () => {
    const { api } = clientReturning(new TypeError('Failed to fetch'));
    const error = await rejection(api.health());
    expect(error).toMatchObject({ code: 'network', status: 0, message: 'Failed to fetch' });
  });

  it('rejects a 200 response that is not JSON', async () => {
    const { api } = clientReturning(new Response('<!doctype html>', { status: 200 }));
    const error = await rejection(api.health());
    expect(error).toMatchObject({ code: 'invalid_response', status: 200 });
  });

  it.each([null, 42, 'ok', [1, 2]])(
    'rejects a 200 JSON body that is not an object: %j',
    async (body) => {
      const { api } = clientReturning(jsonResponse(body));
      const error = await rejection(api.health());
      expect(error).toMatchObject({ code: 'invalid_response', status: 200 });
    },
  );

  it('percent-encodes unit and check names', async () => {
    const { fetch, api } = clientReturning(jsonResponse(fixtures['check-detail.json']));
    await api.check('Game port/1');
    expect(requestedUrl(fetch)).toBe('/api/v1/checks/Game%20port%2F1');
  });

  it('builds query strings with repeated keys and skips unset values', async () => {
    const { fetch, api } = clientReturning(jsonResponse(fixtures['metrics-query.json']));
    await api.metricsQuery({ keys: ['cpu.usage', 'net.rx:*'], from: 100, points: 300 });
    expect(requestedUrl(fetch)).toBe(
      '/api/v1/metrics/query?key=cpu.usage&key=net.rx%3A*&from=100&points=300',
    );
  });

  it('omits the query string when no options are given', async () => {
    const { fetch, api } = clientReturning(jsonResponse(fixtures['events.json']));
    await api.events();
    expect(requestedUrl(fetch)).toBe('/api/v1/events');
  });

  it('uses the global fetch by default', async () => {
    const fetch = vi.fn(async () => jsonResponse(fixtures['health.json']));
    vi.stubGlobal('fetch', fetch);
    try {
      await createClient().health();
      expect(fetch).toHaveBeenCalledOnce();
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

describe('aborting', () => {
  // Behaves like the real fetch: never settles on its own, rejects with the signal's reason.
  const hangingFetch = vi.fn<typeof globalThis.fetch>(
    (_input, init) =>
      new Promise((_resolve, reject) => {
        init?.signal?.addEventListener('abort', () => reject(init.signal?.reason));
      }),
  );

  it('passes the signal to fetch and rejects with an AbortError, not an ApiRequestError', async () => {
    const controller = new AbortController();
    const pending = createClient({ fetch: hangingFetch }).overview({ signal: controller.signal });
    controller.abort();
    const error: unknown = await pending.catch((e: unknown) => e);
    expect(hangingFetch.mock.calls[0]?.[1]?.signal).toBe(controller.signal);
    expect(error).not.toBeInstanceOf(ApiRequestError);
    expect(isAbortError(error)).toBe(true);
  });

  it('treats an abort while reading the body as an abort', async () => {
    const controller = new AbortController();
    const response = jsonResponse({});
    vi.spyOn(response, 'json').mockImplementation(async () => {
      controller.abort();
      throw new DOMException('aborted', 'AbortError');
    });
    const api = createClient({ fetch: async () => response });
    const error: unknown = await api.health({ signal: controller.signal }).catch((e: unknown) => e);
    expect(isAbortError(error)).toBe(true);
  });

  it('still reports genuine network failures when a signal is given but not aborted', async () => {
    const api = createClient({ fetch: async () => Promise.reject(new TypeError('offline')) });
    const error = await rejection(api.overview({ signal: new AbortController().signal }));
    expect(error.code).toBe('network');
  });
});

describe('isAbortError', () => {
  it.each([
    [new DOMException('x', 'AbortError'), true],
    [new DOMException('x', 'TimeoutError'), false],
    [new ApiRequestError('network', 'x', 0), false],
    [new Error('AbortError'), false],
  ])('%s -> %s', (value, expected) => {
    expect(isAbortError(value)).toBe(expected);
  });
});

describe('isApiError', () => {
  it.each([
    [fixtures['error.json'], true],
    [{ error: { code: 'x' } }, false],
    [{ error: 'boom' }, false],
    [null, false],
    ['error', false],
  ])('%j -> %s', (value, expected) => {
    expect(isApiError(value)).toBe(expected);
  });
});
