// @vitest-environment jsdom
// Automated accessibility gate (M6.5): every key page, in its normal and problem states, must have
// no serious or critical axe violations. jsdom has no layout, so color contrast is checked by hand
// instead (web/ACCESSIBILITY.md); everything structural (names, roles, landmarks, labels, ARIA) is
// checked here.
import { cleanup, render, screen } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, describe, expect, it, vi } from 'vitest';
import App from './App.svelte';
import { ApiRequestError, type ApiClient } from './lib/api/client';
import { fixtures } from './lib/api/fixtures';
import type { Overview } from './lib/api/types';
import type { Visibility } from './lib/poller';
import { installResizeObserver } from './test/fake-uplot';

vi.mock('uplot', () => import('./test/fake-uplot'));
installResizeObserver();

const visible: Visibility = { isHidden: () => false, subscribe: () => () => {} };

function client(overview: () => Promise<Overview>) {
  return {
    overview,
    metrics: async () => fixtures['metrics.json'],
    metricsQuery: async (_query: Parameters<ApiClient['metricsQuery']>[0]) =>
      fixtures['metrics-query.json'],
    service: async () => fixtures['service-detail.json'],
    timer: async () => fixtures['timer-detail.json'],
    check: async () => fixtures['check-detail.json'],
    events: async () => fixtures['events.json'],
    library: async () => fixtures['library.json'],
  };
}

const normal = () => client(async () => fixtures['overview.json']);

const empty = () =>
  client(async () => ({
    ...fixtures['overview.json'],
    health: 'healthy',
    counts: { healthy: 1, warning: 0, critical: 0, unknown: 0 },
    services: [],
    timers: [],
    checks: [],
    recentEvents: [],
  }));

const offline = () =>
  client(async () => {
    throw new ApiRequestError('network', 'Failed to fetch', 0);
  });

const notFound = () => ({
  ...normal(),
  timer: async () => {
    throw new ApiRequestError('not_found', 'unknown timer', 404);
  },
});

// The `normal` overview has a library card (overview.json configures `[library]`).
const unconfigured = () => ({
  ...client(async () => ({ ...fixtures['overview.json'], library: null })),
  library: async () => fixtures['library-unconfigured.json'],
});

const brokenLibrary = () => ({
  ...normal(),
  library: async () => ({ ...fixtures['library.json'], report: null }),
});

const pending = () => client(() => new Promise<Overview>(() => {}));

async function violations(): Promise<string[]> {
  const result = await axe.run(document.body, {
    resultTypes: ['violations'],
    // No layout engine in jsdom; contrast is on the manual checklist.
    rules: { 'color-contrast': { enabled: false } },
  });
  const minor = result.violations.filter((v) => v.impact !== 'serious' && v.impact !== 'critical');
  if (minor.length > 0) console.info('axe (minor):', minor.map((v) => v.id).join(', '));
  return result.violations
    .filter((v) => v.impact === 'serious' || v.impact === 'critical')
    .map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(' ')).join(', ')})`);
}

describe('accessibility (axe)', () => {
  afterEach(() => {
    cleanup();
    location.hash = '';
  });

  it.each([
    ['overview', '', normal, 'heading'],
    ['overview, nothing configured', '', empty, 'heading'],
    ['overview, server offline on first load', '', offline, 'alert'],
    ['overview, loading', '', pending, 'status'],
    ['system', '#/system', normal, 'heading'],
    ['service detail', '#/services/minecraft.service', normal, 'heading'],
    ['timer detail', '#/timers/music-sync.timer', normal, 'heading'],
    ['check detail', '#/checks/Game%20port', normal, 'heading'],
    ['events', '#/events', normal, 'heading'],
    ['library', '#/library', normal, 'heading'],
    ['library, filtered', '#/library?severity=error', normal, 'heading'],
    ['library, no valid report', '#/library', brokenLibrary, 'heading'],
    ['library, not configured', '#/library', unconfigured, 'heading'],
    ['overview without a library', '', unconfigured, 'heading'],
    ['not found', '#/nowhere', normal, 'heading'],
    ['unknown timer', '#/timers/nope.timer', notFound, 'heading'],
  ] as const)('%s has no serious or critical violations', async (_name, hash, make, ready) => {
    location.hash = hash;
    render(App, { client: make(), visibility: visible });
    await screen.findAllByRole(ready);
    // Let every feed settle (pages, charts, sparklines).
    await new Promise((r) => setTimeout(r, 20));
    expect(await violations()).toEqual([]);
  });
});
