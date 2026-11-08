import { describe, expect, it, vi } from 'vitest';
import { ApiRequestError } from './api/client';
import { watchUnauthorized } from './auth';

describe('watchUnauthorized', () => {
  it('reports a 401 from any method and still rejects with it', async () => {
    const onUnauthorized = vi.fn();
    const error = new ApiRequestError('unauthorized', 'auth required', 401);
    const client = watchUnauthorized(
      {
        ok: async (n: number) => n * 2,
        denied: async () => {
          throw error;
        },
      },
      onUnauthorized,
    );

    await expect(client.ok(21)).resolves.toBe(42);
    expect(onUnauthorized).not.toHaveBeenCalled();
    await expect(client.denied()).rejects.toBe(error);
    expect(onUnauthorized).toHaveBeenCalledOnce();
  });

  it('ignores other failures', async () => {
    const onUnauthorized = vi.fn();
    const client = watchUnauthorized(
      {
        fails: async () => {
          throw new ApiRequestError('internal', 'boom', 500);
        },
      },
      onUnauthorized,
    );
    await expect(client.fails()).rejects.toThrow('boom');
    expect(onUnauthorized).not.toHaveBeenCalled();
  });
});
