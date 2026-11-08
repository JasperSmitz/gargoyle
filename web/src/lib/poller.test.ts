import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PollTimeoutError, createPoller, type Visibility } from './poller';

class FakeVisibility implements Visibility {
  hidden = false;
  private listeners = new Set<() => void>();
  isHidden = () => this.hidden;
  subscribe(onChange: () => void) {
    this.listeners.add(onChange);
    return () => this.listeners.delete(onChange);
  }
  set(hidden: boolean) {
    this.hidden = hidden;
    this.listeners.forEach((l) => l());
  }
  get listenerCount() {
    return this.listeners.size;
  }
}

/** A task whose calls can be resolved or rejected by hand and that honors its abort signal. */
function controllableTask() {
  const calls: {
    signal: AbortSignal;
    resolve: (v: number) => void;
    reject: (e: unknown) => void;
  }[] = [];
  const task = vi.fn(
    (signal: AbortSignal) =>
      new Promise<number>((resolve, reject) => {
        calls.push({ signal, resolve, reject });
        signal.addEventListener('abort', () => reject(signal.reason));
      }),
  );
  return { task, calls };
}

const INTERVAL = 10_000;

describe('createPoller', () => {
  let visibility: FakeVisibility;
  const onSuccess = vi.fn();
  const onError = vi.fn();

  beforeEach(() => {
    vi.useFakeTimers();
    visibility = new FakeVisibility();
    onSuccess.mockReset();
    onError.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  function setup(task: (signal: AbortSignal) => Promise<number>, timeoutMs?: number) {
    return createPoller({ task, intervalMs: INTERVAL, timeoutMs, onSuccess, onError, visibility });
  }

  it('polls immediately, then once per interval after each response', async () => {
    let n = 0;
    const task = vi.fn(async () => ++n);
    const poller = setup(task);
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(onSuccess).toHaveBeenLastCalledWith(1);

    await vi.advanceTimersByTimeAsync(INTERVAL - 1);
    expect(task).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(onSuccess).toHaveBeenLastCalledWith(2);

    await vi.advanceTimersByTimeAsync(INTERVAL * 3);
    expect(task).toHaveBeenCalledTimes(5);
    poller.stop();
  });

  it('never overlaps requests: the interval starts when the previous one settles', async () => {
    const { task, calls } = controllableTask();
    const poller = setup(task, INTERVAL * 10);
    poller.start();
    await vi.advanceTimersByTimeAsync(INTERVAL * 3);
    expect(task).toHaveBeenCalledTimes(1);

    calls[0]?.resolve(1);
    await vi.advanceTimersByTimeAsync(INTERVAL);
    expect(task).toHaveBeenCalledTimes(2);
    poller.stop();
  });

  it('reports errors and keeps polling', async () => {
    const task = vi
      .fn<(s: AbortSignal) => Promise<number>>()
      .mockRejectedValueOnce(new Error('down'))
      .mockResolvedValueOnce(7);
    const poller = setup(task);
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(onError).toHaveBeenCalledWith(new Error('down'));

    await vi.advanceTimersByTimeAsync(INTERVAL);
    expect(onSuccess).toHaveBeenCalledWith(7);
    poller.stop();
  });

  it('aborts a request that exceeds the timeout and reports a timeout error', async () => {
    const { task, calls } = controllableTask();
    const poller = setup(task, 3_000);
    poller.start();
    await vi.advanceTimersByTimeAsync(3_000);
    expect(calls[0]?.signal.aborted).toBe(true);
    expect(onError).toHaveBeenCalledOnce();
    expect(onError.mock.calls[0]?.[0]).toBeInstanceOf(PollTimeoutError);

    await vi.advanceTimersByTimeAsync(INTERVAL);
    expect(task).toHaveBeenCalledTimes(2);
    poller.stop();
  });

  it('pauses while hidden, aborting silently, and refreshes when visible again', async () => {
    const { task, calls } = controllableTask();
    const poller = setup(task);
    poller.start();
    await vi.advanceTimersByTimeAsync(0);

    visibility.set(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(calls[0]?.signal.aborted).toBe(true);
    expect(onError).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(INTERVAL * 5);
    expect(task).toHaveBeenCalledTimes(1);

    visibility.set(false);
    expect(task).toHaveBeenCalledTimes(2);
    calls[1]?.resolve(42);
    await vi.advanceTimersByTimeAsync(0);
    expect(onSuccess).toHaveBeenCalledWith(42);
    poller.stop();
  });

  it('does not poll on start while hidden', async () => {
    visibility.hidden = true;
    const task = vi.fn(async () => 1);
    const poller = setup(task);
    poller.start();
    await vi.advanceTimersByTimeAsync(INTERVAL * 2);
    expect(task).not.toHaveBeenCalled();
    poller.stop();
  });

  it('stop aborts silently, schedules nothing and unsubscribes', async () => {
    const { task, calls } = controllableTask();
    const poller = setup(task);
    poller.start();
    expect(visibility.listenerCount).toBe(1);
    poller.stop();
    await vi.advanceTimersByTimeAsync(INTERVAL * 3);

    expect(calls[0]?.signal.aborted).toBe(true);
    expect(task).toHaveBeenCalledTimes(1);
    expect(onError).not.toHaveBeenCalled();
    expect(visibility.listenerCount).toBe(0);
  });

  it('refresh supersedes the request in flight; its late result is ignored', async () => {
    const results: ((v: number) => void)[] = [];
    // Ignores the signal, to prove superseded results are dropped even if they still arrive.
    const task = vi.fn(() => new Promise<number>((resolve) => results.push(resolve)));
    const poller = setup(task, INTERVAL * 10);
    poller.start();
    poller.refresh();
    expect(task).toHaveBeenCalledTimes(2);

    results[0]?.(1);
    results[1]?.(2);
    await vi.advanceTimersByTimeAsync(0);
    expect(onSuccess.mock.calls).toEqual([[2]]);
    poller.stop();
  });

  it('does not report a consumer exception as a failed request, and keeps polling', async () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    onSuccess.mockImplementationOnce(() => {
      throw new Error('render bug');
    });
    const task = vi.fn(async () => 1);
    const poller = setup(task);
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(onError).not.toHaveBeenCalled();
    expect(consoleError).toHaveBeenCalledOnce();

    await vi.advanceTimersByTimeAsync(INTERVAL);
    expect(task).toHaveBeenCalledTimes(2);
    expect(onSuccess).toHaveBeenCalledTimes(2);
    poller.stop();
    consoleError.mockRestore();
  });

  it('start is idempotent', async () => {
    const task = vi.fn(async () => 1);
    const poller = setup(task);
    poller.start();
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(task).toHaveBeenCalledTimes(1);
    expect(visibility.listenerCount).toBe(1);
    poller.stop();
  });
});
