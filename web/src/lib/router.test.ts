import { describe, expect, it } from 'vitest';
import { href, matchPattern, parseHash } from './router';

describe('parseHash', () => {
  it.each(['', '#', '#/', '#//', '#/?x=1'])('%j is the overview', (hash) => {
    expect(parseHash(hash)).toEqual({ name: 'overview', params: {}, path: '/' });
  });

  it('reports unknown paths as not-found, normalised', () => {
    expect(parseHash('#/nope/')).toEqual({ name: 'not-found', params: {}, path: '/nope' });
  });
});

describe('matchPattern', () => {
  it('extracts and decodes params', () => {
    expect(matchPattern('/services/:unit', '/services/my%20app.service')).toEqual({
      unit: 'my app.service',
    });
  });

  it.each([
    ['/services/:unit', '/services'],
    ['/services/:unit', '/services/a/b'],
    ['/services/:unit', '/timers/a'],
    ['/services/:unit', '/services/%E0'],
  ])('%s does not match %s', (pattern, path) => {
    expect(matchPattern(pattern, path)).toBeNull();
  });
});

describe('href', () => {
  it('encodes each segment so it round-trips through matchPattern', () => {
    const link = href('checks', 'Music / HTTP');
    expect(link).toBe('#/checks/Music%20%2F%20HTTP');
    expect(matchPattern('/checks/:name', link.slice(1))).toEqual({ name: 'Music / HTTP' });
  });
});
