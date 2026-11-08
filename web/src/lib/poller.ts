// Polls a task on a fixed interval. Framework-agnostic so it can be tested with fake timers.
//
// - The next poll is scheduled when the previous one settles (setTimeout, not setInterval), so a
//   slow server never gets overlapping requests.
// - While the page is hidden nothing is fetched; the in-flight request is aborted and a fresh
//   poll runs as soon as the page is visible again.
// - A request that outlives `timeoutMs` is aborted and reported as an error. Aborts the poller
//   causes itself (stop, hide, refresh) are never reported.

import { isAbortError } from './api/client';

export interface Visibility {
  isHidden(): boolean;
  /** Calls `onChange` whenever visibility changes; returns an unsubscribe function. */
  subscribe(onChange: () => void): () => void;
}

export interface PollerOptions<T> {
  task: (signal: AbortSignal) => Promise<T>;
  intervalMs: number;
  /** Defaults to `intervalMs`. */
  timeoutMs?: number;
  onSuccess: (value: T) => void;
  onError: (error: unknown) => void;
  /** Defaults to the page visibility of `document`. */
  visibility?: Visibility;
}

export interface Poller {
  /** Polls immediately (if visible) and then every interval. Idempotent. */
  start(): void;
  /** Aborts any request in flight and stops scheduling. */
  stop(): void;
  /** Polls now, abandoning any request in flight, and restarts the interval from there. */
  refresh(): void;
}

export class PollTimeoutError extends Error {
  constructor(timeoutMs: number) {
    super(`no response within ${Math.round(timeoutMs / 1000)}s`);
    this.name = 'PollTimeoutError';
  }
}

export function documentVisibility(doc: Document = document): Visibility {
  return {
    isHidden: () => doc.visibilityState === 'hidden',
    subscribe(onChange) {
      doc.addEventListener('visibilitychange', onChange);
      return () => doc.removeEventListener('visibilitychange', onChange);
    },
  };
}

export function createPoller<T>(options: PollerOptions<T>): Poller {
  const { task, intervalMs, onSuccess, onError } = options;
  const timeoutMs = options.timeoutMs ?? intervalMs;
  let visibility: Visibility | undefined;

  let running = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let inFlight: AbortController | undefined;
  let unsubscribe: (() => void) | undefined;

  function cancel() {
    clearTimeout(timer);
    timer = undefined;
    inFlight?.abort();
    inFlight = undefined;
  }

  async function poll() {
    cancel();
    const controller = new AbortController();
    inFlight = controller;
    let timedOut = false;
    const deadline = setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, timeoutMs);

    let outcome: { ok: true; value: T } | { ok: false; error: unknown };
    try {
      outcome = { ok: true, value: await task(controller.signal) };
    } catch (error) {
      outcome = { ok: false, error };
    } finally {
      clearTimeout(deadline);
    }
    if (inFlight !== controller) return; // superseded while the response was on its way

    inFlight = undefined;
    if (running && !visibility?.isHidden()) timer = setTimeout(poll, intervalMs);

    // Callbacks run outside the request's try: a consumer bug is not a failed request, and must
    // not stop polling either.
    try {
      if (outcome.ok) onSuccess(outcome.value);
      else if (timedOut) onError(new PollTimeoutError(timeoutMs));
      else if (!isAbortError(outcome.error)) onError(outcome.error);
    } catch (error) {
      console.error('poller callback failed', error);
    }
  }

  function onVisibilityChange() {
    if (!running) return;
    if (visibility?.isHidden()) cancel();
    else void poll();
  }

  return {
    start() {
      if (running) return;
      running = true;
      visibility = options.visibility ?? documentVisibility();
      unsubscribe = visibility.subscribe(onVisibilityChange);
      if (!visibility.isHidden()) void poll();
    },
    stop() {
      running = false;
      cancel();
      unsubscribe?.();
      unsubscribe = undefined;
    },
    refresh() {
      if (running && !visibility?.isHidden()) void poll();
    },
  };
}
