// A polled value with reactive loading and error state, for chart data and the series list.
// On failure the last good value is kept, so charts keep their frame and show a note instead of
// blanking.

import { createPoller, type Poller, type Visibility } from './poller';

/** ARCHITECTURE §6: charts refresh every 60s. */
export const CHART_POLL_MS = 60_000;

/** Unit state changes once per `units_interval` (default 30s), so detail pages poll that often. */
export const DETAIL_POLL_MS = 30_000;

export interface PolledFeedOptions {
  intervalMs?: number;
  visibility?: Visibility;
}

export class PolledFeed<T> {
  data = $state<T | null>(null);
  /** The latest poll's error; null once a poll succeeds again. */
  error = $state<unknown>(null);
  /** True while a request is in flight, including refreshes over existing data. */
  loading = $state(false);

  readonly #poller: Poller;

  constructor(load: (signal: AbortSignal) => Promise<T>, options: PolledFeedOptions = {}) {
    this.#poller = createPoller({
      task: (signal) => {
        this.loading = true;
        return load(signal);
      },
      intervalMs: options.intervalMs ?? CHART_POLL_MS,
      visibility: options.visibility,
      onSuccess: (value) => {
        this.data = value;
        this.error = null;
        this.loading = false;
      },
      onError: (error) => {
        this.error = error;
        this.loading = false;
      },
    });
  }

  start() {
    this.#poller.start();
  }

  stop() {
    this.#poller.stop();
    this.loading = false;
  }

  /** Polls now, e.g. after the range changed; the old data stays visible until the new arrives. */
  refresh() {
    this.#poller.refresh();
  }
}
