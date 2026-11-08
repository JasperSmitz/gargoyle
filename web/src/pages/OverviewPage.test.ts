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
      '4 of 6 targets need attention',
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
    // counts are unchanged ({critical:2, warning:1, unknown:1, healthy:2}): the system is one target.
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe(
      '4 of 6 targets need attention',
    );
    const problems = screen.getByRole('list', { name: 'Needs attention' });
    expect(within(problems).getAllByRole('listitem')).toHaveLength(6);
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
    const timers = within(section('Timers')).getAllByRole('listitem');
    // The message explains the failure once; the meta line keeps only the timing.
    expect(timers[0]?.textContent?.match(/exit-code \(status 1\)/g)).toHaveLength(1);
    expect(timers[0]?.textContent).toContain('last 2h ago');
    expect(timers[0]?.textContent).toContain('next in 21h');
    expect(timers[1]?.textContent?.match(/never run/g)).toHaveLength(1);

    expect(section('Checks').textContent).toContain('3.1 ms');
    const events = within(section('Recent events')).getAllByRole('listitem');
    expect(events).toHaveLength(3);
    expect(events[0]?.textContent).toContain('minecraft.service');
    expect(events[0]?.textContent).toMatch(/Healthy\s*→\s*Critical/);
  });

  it('shows an empty state with a config hint and hides empty optional sections', () => {
    const empty = structuredClone(example);
    empty.services = [];
    empty.timers = [];
    empty.checks = [];
    empty.recentEvents = [];
    renderOverview(empty);
    const services = section('Services');
    expect(services.textContent).toContain('No services configured');
    expect(services.textContent).toContain('[[service]]');
    expect(screen.queryByRole('region', { name: /^Timers/ })).toBeNull();
    expect(screen.queryByRole('region', { name: /^Checks/ })).toBeNull();
    expect(screen.queryByRole('region', { name: /^Recent events/ })).toBeNull();
  });

  it('reports an all-healthy host plainly', () => {
    const healthy = structuredClone(example);
    healthy.health = 'healthy';
    healthy.system.health = 'healthy';
    healthy.system.disks.forEach((d) => (d.health = 'healthy'));
    healthy.services.forEach((s) => ((s.health = 'healthy'), (s.message = null)));
    healthy.timers.forEach((t) => ((t.health = 'healthy'), (t.message = null)));
    healthy.counts = { healthy: 6, warning: 0, critical: 0, unknown: 0 };
    renderOverview(healthy);
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('All 6 targets healthy');
    expect(screen.queryByRole('list', { name: 'Needs attention' })).toBeNull();
  });
});
