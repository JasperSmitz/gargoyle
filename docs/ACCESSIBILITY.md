# Accessibility and CSP checklist

Automated checks cover structure; this list covers what needs eyes and a keyboard. Run it before
a release and after any change to layout, tokens or navigation.

## Automated (runs in `make check`)

- `src/a11y.test.ts`: axe-core in jsdom on the overview (normal, nothing configured, offline
  on first load, loading, without a library), System, service, timer and check detail, an
  unknown timer, Library (normal, filtered, no valid report, not configured), Events and
  not-found. Fails on any **serious** or **critical** violation. jsdom has no layout, so
  `color-contrast` is disabled there and checked by hand below.
- `src/csp.test.ts`: no inline `<script>`, no inline `style="…"` attributes, no runtime `<style>`
  elements, no `eval`/`new Function`, and nothing loaded from another origin (fonts included).
- `pnpm size` (after `pnpm build`): total gzipped JS must stay at or under 150 KiB.

## Keyboard

Use a real browser, no mouse. Do it at desktop width and at 360px.

- [ ] The first <kbd>Tab</kbd> shows a **Skip to content** link. <kbd>Enter</kbd> moves focus to
      the page heading, and the URL does not change.
- [ ] Every focused element has a visible ring (2px accent outline) in light and dark mode,
      and the ring is not clipped by a card or table edge.
- [ ] Tab order follows the page: brand, main nav, page content top to bottom. Nothing focusable
      is hidden or off-screen.
- [ ] Overview: the "needs attention" links, every service, timer and check name, and the
      event targets can be reached and opened with <kbd>Enter</kbd>.
- [ ] Following a link to another page moves focus to that page's `h1` (or the main region
      while it loads), and the tab title names the page (`System · Gargoyle — host`).
- [ ] Changing the range (1h … 1y) keeps focus on the range link. Changing an Events filter
      keeps focus on that select.
- [ ] Charts: **Show table** is reachable, toggles with <kbd>Enter</kbd>/<kbd>Space</kbd>, and
      announces its expanded state.
- [ ] Overview: the library card's link and the library row in "needs attention" open the
      Library page with <kbd>Enter</kbd>. Without `[library]` there is no Library nav entry.
- [ ] Library: tab order is the severity filter (All, Warnings and errors, Errors only),
      **Expand all**, **Collapse all**, then each album summary. The current filter is marked
      (`aria-current`); choosing one with <kbd>Enter</kbd> updates the URL (`?severity=…`) and
      keeps focus on it.
- [ ] Library: each album summary opens and closes with <kbd>Enter</kbd> and <kbd>Space</kbd>,
      and **Expand all**/**Collapse all** open and close every shown album without moving focus.
      Issue severities show an icon and text, not color alone. At 360px the paths wrap and the
      page has no horizontal scroll.
- [ ] Events: **Load older events** is reachable. Offline banner: **Retry now** is reachable.
      401 banner: **Reload** is reachable.

## Contrast and color

Check both themes (`prefers-color-scheme` light and dark), for example with the browser's
DevTools color picker or axe DevTools in a real browser, where layout exists.

- [ ] Body text and secondary text are at least 4.5:1 on their actual background. Note that
      `--color-text-faint` is only AA on bg, surface and surface-raised: on `surface-sunken`
      use `--color-text-muted`.
- [ ] Health text (`--health-*-fg`) is at least 4.5:1 on surfaces and on its own `-bg` badge fill.
- [ ] The focus ring and UI boundaries (inputs, buttons, the range picker) are at least 3:1.
- [ ] Health is never shown by color alone: every badge, row edge and chart status has an icon or
      text label next to it.
- [ ] Chart series are identified by legend and table as well as by color (categorical slots 3–5
      are below 3:1 on the light surface by design, see tokens.css).

## Motion, zoom and print

- [ ] With "reduce motion" enabled in the OS, the loading skeletons do not pulse and nothing
      animates.
- [ ] At 200% browser zoom (and at 360px width) there is no horizontal page scroll, and no text
      is cut off.
- [ ] Print preview (also from dark mode) is in light ink and leaves out the nav, range pickers
      and buttons.

## Screen reader spot check

With Orca (Linux) or VoiceOver:

- [ ] Each page has exactly one `h1`. Section headings are `h2` and card titles are `h3`.
- [ ] Loading states are announced as "Loading overview" (or the page's own noun), not as a
      wall of empty boxes.
- [ ] The offline banner and the 401 message are announced once, as alerts, and the ticking
      "Showing data from …" age is not re-announced every second.
- [ ] Relative times ("3h ago") expose the absolute time through `<time datetime>` and a tooltip.

## CSP

The backend sends
`default-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'none'; object-src 'none'`.
That policy allows no inline scripts and no inline styles.

- [ ] Serve a release build with that header, load every page and an offline state, and check
      that the console shows no "Refused to …" CSP errors.
- [ ] Dynamic styles go through Svelte's `style:` directive or `element.style` (CSSOM, which the
      policy allows). Never write a `style="…"` attribute in markup or call
      `setAttribute('style', …)`.
- [ ] The favicon stays a `data:` URI (allowed by `img-src data:`), and no web fonts are loaded.
