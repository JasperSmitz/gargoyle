// With auth enabled the browser sends Basic credentials it was given once; when they stop working
// (password changed, session forgotten) every request answers 401. The app then says so once,
// prominently, instead of each panel showing a generic failure.

import { ApiRequestError } from './api/client';

export function isUnauthorized(error: unknown): boolean {
  return error instanceof ApiRequestError && error.status === 401;
}

type AsyncMethods = Record<string, (...args: never[]) => Promise<unknown>>;

/**
 * Wraps every method of an API client so a 401 from any of them calls `onUnauthorized` (and
 * still rejects, so the caller's own error handling is unchanged).
 */
export function watchUnauthorized<C extends AsyncMethods>(
  client: C,
  onUnauthorized: () => void,
): C {
  const wrapped: Record<string, unknown> = {};
  for (const [name, method] of Object.entries(client)) {
    wrapped[name] = async (...args: never[]) => {
      try {
        return await method(...args);
      } catch (error) {
        if (isUnauthorized(error)) onUnauthorized();
        throw error;
      }
    };
  }
  // Same keys, each with the same signature and behavior; only the side effect is added.
  return wrapped as C;
}
