import { describe, expect, it } from 'vitest';
import { ApiRequestError } from './api/client';
import { describeFeedError } from './overview-feed.svelte';
import { PollTimeoutError } from './poller';

describe('describeFeedError', () => {
  it.each([
    [new ApiRequestError('network', 'Failed to fetch', 0), 'Server unreachable'],
    [new PollTimeoutError(10_000), 'Server not responding'],
    [new ApiRequestError('unauthorized', 'auth required', 401), 'Authentication required'],
    [new ApiRequestError('internal', 'boom', 500), 'Server error'],
    [new Error('weird'), 'Update failed'],
  ])('%s -> %s', (error, title) => {
    expect(describeFeedError(error).title).toBe(title);
  });

  it('includes the HTTP status for server errors', () => {
    expect(describeFeedError(new ApiRequestError('internal', 'boom', 500)).detail).toBe(
      'boom (HTTP 500)',
    );
  });
});
