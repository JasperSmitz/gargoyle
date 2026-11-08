// Live `/overview` data for the whole app (the header needs it as well as the overview page).
// On failure the last good data is kept and `error` is set, so the UI can show a stale banner
// over still-useful numbers instead of blanking the page.

import { ApiRequestError } from './api/client';
import type { Overview } from './api/types';
import { isTimestamp } from './format';
import { PollTimeoutError, createPoller, type Poller, type Visibility } from './poller';

/** ARCHITECTURE §6: the dashboard home page is polled every 10s. */
export const OVERVIEW_POLL_MS = 10_000;

export interface FeedOptions {
  intervalMs?: number;
  visibility?: Visibility;
  /** Client clock in unix seconds; injectable for tests. */
  clock?: () => number;
}

export class OverviewFeed {
  data = $state<Overview | null>(null);
  /** The latest poll's error; null once a poll succeeds again. */
  error = $state<unknown>(null);
  /** Client time (unix seconds) when `data` arrived. */
  receivedAt = $state<number | null>(null);
  /**
   * Server clock minus client clock at the last response. Server timestamps (events, timer runs)
   * are compared against `clientNow + clockOffset`, so a skewed client clock does not turn
   * "in 5m" into "2h ago".
   */
  clockOffset = $state(0);

  readonly #poller: Poller;
  readonly #clock: () => number;

  constructor(load: (signal: AbortSignal) => Promise<Overview>, options: FeedOptions = {}) {
    this.#clock = options.clock ?? (() => Date.now() / 1000);
    this.#poller = createPoller({
      task: load,
      intervalMs: options.intervalMs ?? OVERVIEW_POLL_MS,
      visibility: options.visibility,
      onSuccess: (overview) => {
        const now = this.#clock();
        this.data = overview;
        this.receivedAt = now;
        // `ts = 0` means the backend has no clock reading to offer; keep the previous offset.
        if (isTimestamp(overview.ts)) this.clockOffset = overview.ts - now;
        this.error = null;
      },
      onError: (error) => {
        this.error = error;
      },
    });
  }

  start() {
    this.#poller.start();
  }

  stop() {
    this.#poller.stop();
  }

  refresh() {
    this.#poller.refresh();
  }
}

/** A short headline and detail for the stale/offline banner. */
export function describeFeedError(error: unknown): { title: string; detail: string } {
  if (error instanceof PollTimeoutError) {
    return { title: 'Server not responding', detail: error.message };
  }
  if (error instanceof ApiRequestError) {
    if (error.status === 0) return { title: 'Server unreachable', detail: error.message };
    if (error.status === 401) {
      return { title: 'Authentication required', detail: 'Reload to sign in' };
    }
    return { title: 'Server error', detail: `${error.message} (HTTP ${error.status})` };
  }
  return { title: 'Update failed', detail: error instanceof Error ? error.message : String(error) };
}
