// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import indexHtml from '../../index.html?raw';
import { fixtures } from './api/fixtures';
import type { Health, Overview } from './api/types';
import { documentTitle, faviconHref, faviconSvg, setFavicon } from './document-health';

const example = fixtures['overview.json'];

function overviewWith(health: Health, counts: Partial<Overview['counts']>): Overview {
  return {
    ...example,
    health,
    counts: { healthy: 3, warning: 0, critical: 0, unknown: 0, ...counts },
  };
}

describe('documentTitle', () => {
  it.each([
    ['healthy', {}, 'Gargoyle — debian-box'],
    ['warning', { warning: 2 }, '⚠ 2 warnings · Gargoyle — debian-box'],
    ['warning', { warning: 1 }, '⚠ 1 warning · Gargoyle — debian-box'],
    ['critical', { critical: 1, warning: 4 }, '✖ 1 critical · Gargoyle — debian-box'],
    ['unknown', { unknown: 3 }, '? 3 unknown · Gargoyle — debian-box'],
    // Never "0 warnings" even if the counts disagree with the aggregate.
    ['warning', {}, '⚠ 1 warning · Gargoyle — debian-box'],
  ] as const)('%s with %o reads %s', (health, counts, title) => {
    expect(documentTitle(overviewWith(health, counts), false)).toBe(title);
  });

  it('says offline instead of a possibly outdated health', () => {
    expect(documentTitle(overviewWith('critical', { critical: 2 }), true)).toBe(
      'Offline · Gargoyle — debian-box',
    );
    expect(documentTitle(null, true)).toBe('Offline · Gargoyle');
  });

  it('is just the app name before the first response', () => {
    expect(documentTitle(null, false)).toBe('Gargoyle');
  });
});

describe('favicon', () => {
  it('matches the static icon in index.html while nothing is known or all is healthy', () => {
    const staticHref = /rel="icon"[\s\S]*?href="([^"]+)"/.exec(indexHtml)?.[1];
    expect(staticHref).toBe(faviconHref(null));
    expect(faviconHref('healthy')).toBe(faviconHref(null));
  });

  it('adds a status dot and recolors the eyes for every other state, one variant each', () => {
    const variants = (['warning', 'critical', 'unknown'] as const).map(faviconHref);
    expect(new Set([faviconHref('healthy'), ...variants]).size).toBe(4);
    for (const health of ['warning', 'critical', 'unknown'] as const) {
      expect(decodeURIComponent(faviconSvg(health)).match(/<circle/g)).toHaveLength(3);
    }
  });

  it('produces well-formed SVG with every # escaped for the data URI', () => {
    for (const health of ['healthy', 'warning', 'critical', 'unknown'] as const) {
      const svg = faviconSvg(health);
      expect(svg).not.toContain('#');
      const doc = new DOMParser().parseFromString(decodeURIComponent(svg), 'image/svg+xml');
      expect(doc.querySelector('parsererror')).toBeNull();
      expect(doc.documentElement.nodeName).toBe('svg');
    }
  });

  it('updates the existing icon link, or adds one', () => {
    document.head.innerHTML = '<link rel="icon" href="old">';
    setFavicon(document, faviconHref('critical'));
    const links = document.head.querySelectorAll('link[rel="icon"]');
    expect(links).toHaveLength(1);
    expect(links[0]?.getAttribute('href')).toBe(faviconHref('critical'));

    document.head.innerHTML = '';
    setFavicon(document, faviconHref('warning'));
    expect(document.head.querySelector('link[rel="icon"]')?.getAttribute('href')).toBe(
      faviconHref('warning'),
    );
  });
});
