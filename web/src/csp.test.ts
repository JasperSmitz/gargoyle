// The backend serves the UI with
//   default-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none';
//   form-action 'none'; object-src 'none'
// i.e. no inline scripts and no inline styles: styling comes from the bundled stylesheet or from
// CSSOM (Svelte's `style:` directive sets element.style), never a `style="…"` attribute in markup.
// These source-level checks keep the app within it; the built bundle is additionally verified in a
// browser with that header (see web/ACCESSIBILITY.md, "CSP").
import { describe, expect, it } from 'vitest';
import indexHtml from '../index.html?raw';

const sources = import.meta.glob(['./**/*.svelte', './**/*.ts', './**/*.css', '!./**/*.test.ts'], {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

describe('CSP compatibility', () => {
  it('index.html has no inline script and only loads the module entry', () => {
    const scripts = [...indexHtml.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/g)];
    expect(scripts).toHaveLength(1);
    expect(scripts[0]?.[1]).toContain('src=');
    expect(scripts[0]?.[2]?.trim()).toBe('');
    expect(indexHtml).not.toMatch(/\son[a-z]+=/i); // inline event handlers
  });

  it('nothing loads from another origin (fonts, scripts, styles, images)', () => {
    // The SVG namespace is an identifier, not a request.
    const external = /(?:https?:)?\/\/(?!www\.w3\.org\/2000\/svg)[a-z0-9.-]+\.[a-z]{2,}/i;
    expect(indexHtml.replace(/<!--[\s\S]*?-->/g, '')).not.toMatch(external);
    for (const [file, text] of Object.entries(sources)) {
      const code = text.replace(/\/\/.*$|\/\*[\s\S]*?\*\/|<!--[\s\S]*?-->/gm, '');
      expect(code, file).not.toMatch(/@import\s+url\(|@font-face/);
      expect(code, file).not.toMatch(/\b(?:src|href)=["'](?:https?:)?\/\//);
    }
  });

  it('no inline style attributes or runtime <style> elements', () => {
    for (const [file, text] of Object.entries(sources)) {
      const markup = text.replace(/<!--[\s\S]*?-->/g, '');
      expect(markup, file).not.toMatch(/<[a-z][^>]*\sstyle=["'{]/i);
      expect(markup, file).not.toMatch(
        /setAttribute\(\s*['"]style['"]|createElement\(\s*['"]style['"]/,
      );
    }
  });

  it('no eval-like code', () => {
    for (const [file, text] of Object.entries(sources)) {
      expect(text, file).not.toMatch(/\beval\(|new Function\(|setTimeout\(\s*['"`]/);
    }
  });
});
