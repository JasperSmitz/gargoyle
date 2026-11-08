// The events page's data: the newest page polled (so new transitions appear), plus older pages
// fetched on demand with the `until` cursor. See events-log.ts for the merge rules.

import { isAbortError } from './api/client';
import type { HealthEvent, UnixSeconds } from './api/types';
import {
  EMPTY_LOG,
  EVENTS_PAGE_SIZE,
  appendOlder,
  mergeNewest,
  type EventsLog,
} from './events-log';
import { DETAIL_POLL_MS } from './feed.svelte';
import { createPoller, type Poller, type Visibility } from './poller';

export type EventsPageLoader = (
  query: { until?: UnixSeconds; limit: number },
  signal: AbortSignal,
) => Promise<HealthEvent[]>;

export interface EventsFeedOptions {
  pageSize?: number;
  intervalMs?: number;
  visibility?: Visibility;
}

export class EventsFeed {
  log = $state<EventsLog>(EMPTY_LOG);
  /** True once the first page has arrived. */
  loaded = $state(false);
  /** The latest poll of the newest page failed; null once one succeeds again. */
  error = $state<unknown>(null);
  loadingOlder = $state(false);
  olderError = $state<unknown>(null);

  readonly #load: EventsPageLoader;
  readonly #pageSize: number;
  readonly #poller: Poller;
  #older: AbortController | undefined;

  constructor(load: EventsPageLoader, options: EventsFeedOptions = {}) {
    this.#load = load;
    this.#pageSize = options.pageSize ?? EVENTS_PAGE_SIZE;
    this.#poller = createPoller({
      task: (signal) => load({ limit: this.#pageSize }, signal),
      intervalMs: options.intervalMs ?? DETAIL_POLL_MS,
      visibility: options.visibility,
      onSuccess: (page) => {
        const next = mergeNewest(this.log, page);
        // Only a restart moves the cursor of a loaded log (see mergeNewest). An older page in
        // flight continues the old log, not this one, and would leave a gap if appended.
        if (this.log.events.length > 0 && next.cursor !== this.log.cursor) this.#cancelOlder();
        this.log = next;
        this.loaded = true;
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
    this.#cancelOlder();
  }

  #cancelOlder() {
    this.#older?.abort();
    this.#older = undefined;
    this.loadingOlder = false;
    this.olderError = null;
  }

  refresh() {
    this.#poller.refresh();
  }

  /** Fetches the next older page; does nothing while one is loading or once the log ended. */
  async loadOlder(): Promise<void> {
    const until = this.log.cursor;
    if (until === null || this.loadingOlder) return;
    const controller = new AbortController();
    this.#older = controller;
    this.loadingOlder = true;
    this.olderError = null;
    try {
      const page = await this.#load({ until, limit: this.#pageSize }, controller.signal);
      // Belt and braces: append only to the log this page continues.
      if (this.#older === controller && this.log.cursor === until) {
        this.log = appendOlder(this.log, page);
      }
    } catch (error) {
      if (this.#older === controller && !isAbortError(error)) this.olderError = error;
    } finally {
      if (this.#older === controller) {
        this.#older = undefined;
        this.loadingOlder = false;
      }
    }
  }
}
