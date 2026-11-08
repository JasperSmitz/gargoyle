import { describe, expect, it } from 'vitest';
import { fixtures } from './api/fixtures';
import { parseEventFilters, targetOptions } from './event-filters';

describe('parseEventFilters', () => {
  it('reads kind and target', () => {
    expect(parseEventFilters(new URLSearchParams('kind=timer&target=a.timer'))).toEqual({
      kind: 'timer',
      target: 'a.timer',
    });
  });

  it('ignores unknown kinds and empty targets', () => {
    expect(parseEventFilters(new URLSearchParams('kind=bogus&target='))).toEqual({
      kind: null,
      target: null,
    });
  });
});

describe('targetOptions', () => {
  it('lists configured targets and ones seen in events, by kind then name', () => {
    const options = targetOptions(fixtures['overview.json'], fixtures['events.json'].events);
    expect(options.map((o) => `${o.kind}:${o.target}`)).toEqual([
      'system:disk:/',
      'service:minecraft.service',
      'service:navidrome.service',
      'timer:backup.timer',
      'timer:music-sync.timer',
      'check:Music HTTP',
      'library:library',
    ]);
  });

  it('offers the library only when it is configured', () => {
    const overview = { ...fixtures['overview.json'], library: null };
    const options = targetOptions(overview, []);
    expect(options.some((o) => o.kind === 'library')).toBe(false);
  });

  it('works before the overview has loaded', () => {
    expect(targetOptions(null, [])).toEqual([]);
  });
});
