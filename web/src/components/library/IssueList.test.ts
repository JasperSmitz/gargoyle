// @vitest-environment jsdom
import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import type { LibraryIssue } from '../../lib/api/types';
import { MAX_BREAK_PARTS } from '../../lib/format';
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
  it('breaks paths at their separators', () => {
    const { container } = render(IssueList, { issues: [issue(['Artist/Album/01.flac'])] });
    const item = container.querySelector('.paths li');
    expect(item?.textContent).toBe('Artist/Album/01.flac');
    expect(item?.querySelectorAll('wbr')).toHaveLength(3);
  });

  // Review M7.5 (Codex P2): a report-controlled path of millions of separators once became
  // millions of <wbr> elements.
  it('keeps the element count bounded for a separator-heavy path', () => {
    const path = 'a/'.repeat(2_000_000);
    const { container } = render(IssueList, { issues: [issue([path])] });
    const item = container.querySelector('.paths li');
    expect(item?.querySelectorAll('wbr').length).toBeLessThanOrEqual(MAX_BREAK_PARTS - 1);
    // Per part: a text node, a <wbr> and Svelte's block anchors; not millions of anything.
    expect(item?.childNodes.length).toBeLessThanOrEqual(5 * MAX_BREAK_PARTS);
    expect(item?.textContent).toBe(path);
  });
});
