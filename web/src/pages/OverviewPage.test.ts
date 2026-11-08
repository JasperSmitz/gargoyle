// @vitest-environment jsdom
import { render, screen, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import { fixtures } from '../lib/api/fixtures';
import type { Overview } from '../lib/api/types';
import OverviewPage from './OverviewPage.svelte';

const example = fixtures['overview.json'];

function renderOverview(overview: Overview = example) {
  return render(OverviewPage, { overview, serverNow: overview.ts });
}

function section(name: string): HTMLElement {
  return screen.getByRole('region', { name: new RegExp(`^${name}`) });
}

describe('OverviewPage', () => {
  it('leads with the overall verdict and the list of problems, worst first', () => {
    renderOverview();
    const summary = screen.getByRole('region', { name: /need attention/ });
    expect(within(summary).getByRole('heading', { level: 1 }).textContent).toBe(
      '5 of 7 targets need attention',
    );
    const problems = within(summary).getByRole('list', { name: 'Needs attention' });
    const rows = within(problems)
      .getAllByRole('listitem')
      .map((li) => li.textContent);
    expect(rows[0]).toContain('Minecraft');
    expect(rows[0]).toContain('unit failed (Result=exit-code)');
    expect(rows.at(-1)).toContain('never run');
  });

  it('counts targets in the headline like the backend, even with several bad system metrics', () => {
    const twoDisks = structuredClone(example);
    twoDisks.system.disks.forEach((d) => (d.health = 'warning'));
    twoDisks.system.cpu.health = 'warning';
    renderOverview(twoDisks);
    // counts are unchanged ({critical:2, warning:2, unknown:1, healthy:2}): the system is one target.
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe(
      '5 of 7 targets need attention',
    );
    const problems = screen.getByRole('list', { name: 'Needs attention' });
    expect(within(problems).getAllByRole('listitem')).toHaveLength(7);
  });

  it("handles the backend's ts = 0 before the first system sample", () => {
    const fresh = structuredClone(example);
    fresh.system.ts = 0;
    fresh.system.health = 'unknown';
    fresh.hostInfo.bootTime = 0;
    renderOverview(fresh);
    const system = section('System');
    expect(within(system).queryByRole('region', { name: 'CPU' })).toBeNull();
    expect(system.textContent).toContain('Waiting for the first system sample');
    expect(within(system).getByRole('region', { name: 'Host' }).textContent).toContain('8d 2h');
    // An epoch-based uptime or age would read as ~20000 days.
    expect(document.body.textContent).not.toMatch(/\d{3,}d\b/);
  });

  it('shows the overall health as icon plus text, not color alone', () => {
    renderOverview();
    const badge = document.querySelector('.summary [data-health="critical"]');
    expect(badge?.textContent?.trim()).toBe('Critical');
    expect(badge?.querySelector('svg[data-icon="octagon"]')).not.toBeNull();
  });

  it('renders the system stat cards, one per disk', () => {
    renderOverview();
    const system = section('System');
    for (const title of ['CPU', 'Load', 'Memory', 'Swap', 'Disk /', 'Disk /srv', 'Host']) {
      expect(within(system).getByRole('region', { name: title })).toBeTruthy();
    }
    const cpu = within(system).getByRole('region', { name: 'CPU' });
    expect(cpu.textContent).toContain('13%');
    expect(cpu.textContent).toContain('iowait 0.4%');

    const load = within(system).getByRole('region', { name: 'Load' });
    expect(load.textContent).toContain('0.42');
    expect(load.textContent).toContain('4 cores · 0.10 per core');

    const root = within(system).getByRole('region', { name: 'Disk /' });
    expect(root.textContent).toContain('84%');
    expect(root.textContent).toContain('Warning');
    expect(within(root).getByRole('meter').getAttribute('aria-valuenow')).toBe('84');

    const host = within(system).getByRole('region', { name: 'Host' });
    expect(host.textContent).toContain('8d 2h');
    expect(host.textContent).toContain('Intel(R) N100 × 4');
  });

  it('renders network and temperature cards from the overview, with temperature health', () => {
    renderOverview();
    const net = screen.getByRole('region', { name: 'Network enp1s0' });
    expect(net.textContent).toContain('118 KiB/s');
    expect(net.textContent).toContain('8.6 KiB/s');
    const temps = screen.getByRole('region', { name: 'Temperatures' });
    expect(temps.textContent).toContain('coretemp/Package id 0');
    expect(temps.textContent).toContain('48.0 °C');
    expect(temps.textContent).toContain('Healthy');
  });

  it('hides the network and temperature cards when those lists are empty', () => {
    const bare = structuredClone(example);
    bare.system.net = [];
    bare.system.temps = [];
    renderOverview(bare);
    expect(screen.queryByRole('region', { name: /^Network/ })).toBeNull();
    expect(screen.queryByRole('region', { name: 'Temperatures' })).toBeNull();
  });

  it('shows the worst sensor health on the temperatures card', () => {
    const hot = structuredClone(example);
    hot.system.temps.push({ label: 'nvme/Composite', celsius: 81, health: 'critical' });
    renderOverview(hot);
    const temps = screen.getByRole('region', { name: 'Temperatures' });
    expect(temps.dataset.health).toBe('critical');
  });

  it('draws last-hour sparklines on the cards it has history for', () => {
    const history = new Map([
      ['cpu.usage', [10, null, 20]],
      ['disk.used:/', [84, 84.1]],
    ]);
    render(OverviewPage, { overview: example, serverNow: example.ts, history });
    expect(
      within(screen.getByRole('region', { name: 'CPU' }))
        .getByRole('img')
        .getAttribute('aria-label'),
    ).toBe('CPU usage, last hour: 10% to 20%');
    expect(within(screen.getByRole('region', { name: 'Disk /' })).getByRole('img')).toBeTruthy();
    expect(within(screen.getByRole('region', { name: 'Memory' })).queryByRole('img')).toBeNull();
  });

  it('says so when there is no swap', () => {
    const noSwap = structuredClone(example);
    noSwap.system.memory.swapTotal = 0;
    renderOverview(noSwap);
    expect(screen.getByRole('region', { name: 'Swap' }).textContent).toContain(
      'No swap configured',
    );
  });

  it('lists services worst first with visible messages for non-healthy ones', () => {
    renderOverview();
    const rows = within(section('Services')).getAllByRole('listitem');
    expect(rows.map((r) => r.dataset.health)).toEqual(['critical', 'healthy']);
    expect(rows[0]?.textContent).toContain('unit failed (Result=exit-code)');
    expect(rows[0]?.textContent).toContain('3 restarts');
    expect(rows[1]?.textContent).toContain('Navidrome');
    expect(rows[1]?.textContent).toContain('145 MiB');
  });

  it('renders timers, checks and recent events when present', () => {
    renderOverview();
    const timers = within(section('Timers')).getAllByRole('row').slice(1);
    // Non-healthy rows always carry the message, even next to the last-run outcome.
    expect(timers[0]?.querySelector('.message')?.textContent).toBe(
      'last run failed: exit-code (status 1)',
    );
    expect(timers[0]?.textContent).toContain('2h ago');
    expect(timers[0]?.textContent).toContain('in 21h 13m');
    expect(timers[1]?.querySelector('.message')?.textContent).toBe('never run');

    expect(section('Checks').textContent).toContain('3.1 ms');
    const events = within(section('Recent events')).getAllByRole('listitem');
    expect(events).toHaveLength(4);
    expect(events[0]?.textContent).toContain('minecraft.service');
    expect(events[0]?.textContent).toMatch(/Healthy\s*→\s*Critical/);
    expect(within(section('Recent events')).getByRole('link', { name: 'All events' })).toBeTruthy();
  });

  it('links services, timers and problems to their detail pages', () => {
    renderOverview();
    const links = (name: string) =>
      screen.getAllByRole('link', { name }).map((a) => a.getAttribute('href'));
    expect(links('Minecraft')).toEqual([
      '#/services/minecraft.service',
      '#/services/minecraft.service',
    ]);
    expect(links('Music sync')).toEqual(['#/timers/music-sync.timer', '#/timers/music-sync.timer']);
    // System problems lead to the charts.
    expect(links('Disk /')).toEqual(['#/system']);
    expect(links('Music HTTP')).toEqual(['#/checks/Music%20HTTP']);
  });

  it('shows a calm "all quiet" state with a config hint when nothing is configured', () => {
    const empty = structuredClone(example);
    empty.services = [];
    empty.timers = [];
    empty.checks = [];
    empty.library = null;
    empty.recentEvents = [];
    empty.health = 'healthy';
    empty.system.health = 'healthy';
    empty.system.disks.forEach((d) => (d.health = 'healthy'));
    empty.counts = { healthy: 1, warning: 0, critical: 0, unknown: 0 };
    renderOverview(empty);
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('System healthy');
    const quiet = section('All quiet');
    for (const kind of ['[[service]]', '[[timer]]', '[[check]]', 'gargoyle.toml']) {
      expect(quiet.textContent).toContain(kind);
    }
    expect(screen.queryByRole('region', { name: /^Services/ })).toBeNull();
    expect(screen.queryByRole('region', { name: /^Timers/ })).toBeNull();
    expect(screen.queryByRole('region', { name: /^Checks/ })).toBeNull();
    expect(section('Recent events').textContent).toContain('No health changes recorded yet');
    // System metrics are still shown.
    const system = screen.getByRole('region', { name: 'System' });
    expect(within(system).getByRole('region', { name: 'CPU' })).toBeTruthy();
  });

  it('does not call a warning "all quiet" when nothing is configured', () => {
    const empty = structuredClone(example);
    empty.services = [];
    empty.timers = [];
    empty.checks = [];
    empty.library = null;
    empty.health = 'warning';
    renderOverview(empty);
    expect(screen.queryByRole('region', { name: /^All quiet/ })).toBeNull();
    expect(screen.getByRole('region', { name: 'Only system metrics are watched' })).toBeTruthy();
  });

  it('names unconfigured kinds in one line instead of empty sections', () => {
    const partial = structuredClone(example);
    partial.timers = [];
    partial.checks = [];
    renderOverview(partial);
    expect(screen.queryByRole('region', { name: /^All quiet/ })).toBeNull();
    expect(section('Services')).toBeTruthy();
    expect(screen.queryByRole('region', { name: /^Timers/ })).toBeNull();
    const hint = screen.getByText(/Not watching any/);
    expect(hint.textContent?.replace(/\s+/g, ' ')).toContain(
      'Not watching any timers or checks. Add [[timer]] or [[check]] entries',
    );
  });

  it('shows checks as a table: kind, sanitized target, latency, last check, failures, health', () => {
    const overview = structuredClone(example);
    overview.checks = structuredClone(fixtures['checks.json'].checks);
    renderOverview(overview);
    const rows = within(section('Checks')).getAllByRole('row');
    expect(rows[0]?.textContent?.replace(/\s+/g, ' ').trim()).toBe(
      'Check Latency Last check Failures Health',
    );
    const cells = (row: HTMLElement | undefined) =>
      [...(row?.querySelectorAll('th, td') ?? [])].map((c) =>
        (c.textContent ?? '').replace(/\s+/g, ' ').trim(),
      );
    // Worst first: the failing TCP check, with its message under the name.
    expect(cells(rows[1])).toEqual([
      'Game port TCP 127.0.0.1:25565 connection refused (1 consecutive failure)',
      'Failed',
      '10s ago',
      '1 in a row',
      'Warning',
    ]);
    expect(cells(rows[2])).toEqual([
      'Music HTTP HTTP http://127.0.0.1:4533/ping',
      '3.1 ms',
      '20s ago',
      '0',
      'Healthy',
    ]);
    expect(within(rows[1]!).getByRole('link', { name: 'Game port' }).getAttribute('href')).toBe(
      '#/checks/Game%20port',
    );
  });

  it('shows the message of every non-healthy target, and names each state in text', () => {
    const overview = structuredClone(example);
    overview.checks = structuredClone(fixtures['checks.json'].checks);
    renderOverview(overview);
    const text = document.body.textContent ?? '';
    for (const target of [...overview.services, ...overview.timers, ...overview.checks]) {
      if (target.health !== 'healthy') expect(text).toContain(target.message);
    }
    // Problem lines use a compact icon; the state is still spoken.
    const problems = screen.getByRole('list', { name: 'Needs attention' });
    const first = within(problems).getAllByRole('listitem')[0]!;
    expect(first.querySelector('.sr-only')?.textContent).toBe('Critical');
    expect(first.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true');
  });

  it('reports an all-healthy host plainly', () => {
    const healthy = structuredClone(example);
    healthy.health = 'healthy';
    healthy.system.health = 'healthy';
    healthy.system.disks.forEach((d) => (d.health = 'healthy'));
    healthy.services.forEach((s) => ((s.health = 'healthy'), (s.message = null)));
    healthy.timers.forEach((t) => ((t.health = 'healthy'), (t.message = null)));
    healthy.library = { ...healthy.library!, health: 'healthy', message: null };
    healthy.counts = { healthy: 7, warning: 0, critical: 0, unknown: 0 };
    renderOverview(healthy);
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('All 7 targets healthy');
    expect(screen.queryByRole('list', { name: 'Needs attention' })).toBeNull();
  });

  it('shows a library card with health, counts, report age and a link', () => {
    renderOverview();
    const card = section('Library');
    const text = card.textContent?.replace(/\s+/g, ' ') ?? '';
    expect(within(card).getByRole('heading', { name: 'Music library' })).toBeTruthy();
    expect(within(card).getByText('Warning')).toBeTruthy();
    expect(text).toContain('2 errors, 4 warnings, 1 info in 95 albums; 1 unreadable file');
    const counts = within(card).getByRole('list', { name: 'Issues by severity' });
    expect(
      within(counts)
        .getAllByRole('listitem')
        .map((li) => li.textContent?.trim()),
    ).toEqual(['2 errors', '4 warnings', '1 info']);
    expect(text).toContain('1 unreadable file');
    expect(text).toContain('95 albums');
    // ts 1794200000 - generatedAt 1794132000 = 68000s.
    expect(text).toContain('Report 18h ago');
    expect(
      within(card).getByRole('link', { name: 'View library report' }).getAttribute('href'),
    ).toBe('#/library');
    // And the library is in the problem list, linking to its page.
    const problems = screen.getByRole('list', { name: 'Needs attention' });
    expect(within(problems).getByRole('link', { name: 'Music library' }).getAttribute('href')).toBe(
      '#/library',
    );
  });

  it('shows chips only for non-zero counts, and "No issues" for a clean library', () => {
    const someWarnings = structuredClone(example);
    someWarnings.library = {
      ...someWarnings.library!,
      issueCounts: { error: 0, warning: 3, info: 0 },
    };
    const first = renderOverview(someWarnings);
    const counts = within(section('Library')).getByRole('list', { name: 'Issues by severity' });
    expect(
      within(counts)
        .getAllByRole('listitem')
        .map((li) => li.textContent?.trim()),
    ).toEqual(['3 warnings']);
    first.unmount();

    const clean = structuredClone(example);
    clean.library = {
      ...clean.library!,
      health: 'healthy',
      message: null,
      issueCounts: { error: 0, warning: 0, info: 0 },
      unreadableFiles: 0,
    };
    renderOverview(clean);
    const card = section('Library');
    expect(within(card).queryByRole('list', { name: 'Issues by severity' })).toBeNull();
    expect(within(card).getByText('No issues')).toBeTruthy();
    expect(card.textContent).not.toMatch(/0 (errors|warnings|info)/);
  });

  it('shows the reason and no counts when the library has no valid report', () => {
    const missing = structuredClone(example);
    missing.library = {
      ...missing.library!,
      health: 'unknown',
      message: 'report not found',
      generatedAt: null,
      age: null,
      issueCounts: null,
      unreadableFiles: null,
      albums: null,
    };
    renderOverview(missing);
    const card = section('Library');
    expect(card.textContent).toContain('report not found');
    expect(within(card).getByText('Unknown')).toBeTruthy();
    expect(within(card).queryByRole('list', { name: 'Issues by severity' })).toBeNull();
    expect(card.textContent).not.toMatch(/unreadable|albums|Report /);
  });

  it('has no library card without [library]', () => {
    renderOverview({ ...structuredClone(example), library: null });
    expect(screen.queryByRole('region', { name: /^Library/ })).toBeNull();
    expect(screen.queryByRole('link', { name: 'View library report' })).toBeNull();
  });

  it('does not say only system metrics are watched when the library is', () => {
    const libraryOnly = structuredClone(example);
    libraryOnly.services = [];
    libraryOnly.timers = [];
    libraryOnly.checks = [];
    renderOverview(libraryOnly);
    expect(screen.queryByRole('region', { name: 'Only system metrics are watched' })).toBeNull();
    expect(screen.queryByRole('region', { name: /^All quiet/ })).toBeNull();
    expect(screen.getByText(/Not watching any/).textContent?.replace(/\s+/g, ' ')).toContain(
      'Not watching any services, timers or checks.',
    );
    expect(section('Library')).toBeTruthy();
  });
});
