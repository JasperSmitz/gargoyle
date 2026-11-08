// @vitest-environment jsdom
import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import type { LibraryIssue } from '../../lib/api/types';
import IssueList from './IssueList.svelte';

function issue(paths: string[]): LibraryIssue {
  return {
    code: 'missing-track',
    severity: 'warning',
    scope: 'album',
    message: 'Track 3 is missing',
    paths,
    pathsTotal: paths.length,
  };
}

describe('IssueList', () => {
  it('renders each path as one text node', () => {
    const { container } = render(IssueList, { issues: [issue(['Artist/Album/01.flac'])] });
    const item = container.querySelector('.paths li');
    expect(item?.textContent?.trim()).toBe('Artist/Album/01.flac');
    expect(item?.querySelectorAll('*')).toHaveLength(0);
  });

  // Review M7.5 (Codex P2, re-review R1): per-path <wbr> break points, even capped per path,
  // gave about 1.6M elements for a full view of 500 issues × 50 separator-heavy paths.
  it('adds no elements per separator for a full view of separator-heavy paths', () => {
    const paths = Array.from({ length: 50 }, () => '/'.repeat(64));
    const issues = Array.from({ length: 500 }, () => issue(paths));
    const { container } = render(IssueList, { issues });
    expect(container.querySelectorAll('wbr')).toHaveLength(0);
    const items = container.querySelectorAll('.paths li');
    expect(items).toHaveLength(25_000);
    for (const item of items) expect(item.children).toHaveLength(0);
  });
});
