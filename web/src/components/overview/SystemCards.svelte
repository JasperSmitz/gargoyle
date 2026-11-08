<script lang="ts">
  import type { HostInfo, SystemMetrics } from '../../lib/api/types';
  import type { Column } from '../../lib/charts/data';
  import { formatValue } from '../../lib/charts/units';
  import { worst } from '../../lib/health';
  import Sparkline from '../charts/Sparkline.svelte';
  import HealthLabel from '../HealthLabel.svelte';
  import {
    formatBytes,
    formatDuration,
    formatLoad,
    formatPercent,
    formatTimestamp,
    isTimestamp,
  } from '../../lib/format';
  import StatCard from '../StatCard.svelte';
  import UsageBar from '../UsageBar.svelte';

  let {
    system,
    hostInfo,
    serverNow,
    history = null,
  }: {
    system: SystemMetrics;
    hostInfo: HostInfo;
    serverNow: number;
    /** Last-hour averages by series key; cards without one simply show no sparkline. */
    history?: Map<string, Column> | null;
  } = $props();

  const trend = (key: string) => history?.get(key) ?? null;
  const tempsHealth = $derived(worst(system.temps.map((t) => t.health)));

  const { cpu, load, memory } = $derived(system);
  const coresText = $derived(
    `${hostInfo.cores} ${hostInfo.cores === 1 ? 'core' : 'cores'}` +
      (hostInfo.cores > 0 ? ` · ${formatLoad(load.one / hostInfo.cores)} per core` : ''),
  );
  const swapPct = $derived(memory.swapTotal > 0 ? (memory.swapUsed / memory.swapTotal) * 100 : 0);
  // The backend sends `system.ts = 0` until the first sample; the metrics are then meaningless.
  const sampled = $derived(isTimestamp(system.ts));
  // `uptime` is as of the response; keep it ticking between polls.
  const uptime = $derived(
    isTimestamp(hostInfo.bootTime)
      ? Math.max(hostInfo.uptime, serverNow - hostInfo.bootTime)
      : hostInfo.uptime,
  );
</script>

{#snippet spark(key: string, unit: 'percent' | 'load', label: string)}
  {@const values = trend(key)}
  {#if values}<Sparkline {values} {unit} {label} />{/if}
{/snippet}

<div class="grid">
  {#if sampled}
    <StatCard title="CPU" health={cpu.health}>
      <p class="value">{formatPercent(cpu.usage)}</p>
      <UsageBar percent={cpu.usage} health={cpu.health} label="CPU usage" />
      {@render spark('cpu.usage', 'percent', 'CPU usage')}
      <p class="detail">iowait {formatPercent(cpu.iowait)}</p>
    </StatCard>

    <StatCard title="Load" health={load.health}>
      <dl class="load">
        <div>
          <dt>1 min</dt>
          <dd class="value">{formatLoad(load.one)}</dd>
        </div>
        <div>
          <dt>5 min</dt>
          <dd>{formatLoad(load.five)}</dd>
        </div>
        <div>
          <dt>15 min</dt>
          <dd>{formatLoad(load.fifteen)}</dd>
        </div>
      </dl>
      {@render spark('load.1', 'load', '1-minute load')}
      <p class="detail">{coresText}</p>
    </StatCard>

    <StatCard title="Memory" health={memory.health}>
      <p class="value">{formatPercent(memory.usedPct)}</p>
      <UsageBar percent={memory.usedPct} health={memory.health} label="Memory usage" />
      {@render spark('mem.used', 'percent', 'Memory usage')}
      <p class="detail">{formatBytes(memory.used)} of {formatBytes(memory.total)}</p>
    </StatCard>

    <!-- The contract carries one health for memory and swap together, so swap shows no badge. -->
    <StatCard title="Swap">
      {#if memory.swapTotal > 0}
        <p class="value">{formatPercent(swapPct)}</p>
        <UsageBar percent={swapPct} label="Swap usage" />
        <p class="detail">{formatBytes(memory.swapUsed)} of {formatBytes(memory.swapTotal)}</p>
      {:else}
        <p class="value muted">None</p>
        <p class="detail">No swap configured</p>
      {/if}
    </StatCard>

    {#each system.disks as disk (disk.mount)}
      <StatCard title="Disk {disk.mount}" subtitle={disk.fsType} health={disk.health}>
        <p class="value">{formatPercent(disk.usedPct)}</p>
        <UsageBar percent={disk.usedPct} health={disk.health} label="Disk {disk.mount} usage" />
        {@render spark(`disk.used:${disk.mount}`, 'percent', `Disk ${disk.mount} usage`)}
        <p class="detail">
          {formatBytes(disk.used)} of {formatBytes(disk.total)} · {formatBytes(
            disk.total - disk.used,
          )} free
        </p>
      </StatCard>
    {/each}

    <!-- Rates have no health in the contract; an empty list means not available (§6). -->
    {#each system.net as net (net.iface)}
      <StatCard title="Network {net.iface}">
        <dl class="rates">
          <div>
            <dt>Received</dt>
            <dd class="value">{formatValue('bytes_per_s', net.rxBps)}</dd>
          </div>
          <div>
            <dt>Sent</dt>
            <dd>{formatValue('bytes_per_s', net.txBps)}</dd>
          </div>
        </dl>
      </StatCard>
    {/each}

    {#if system.temps.length > 0}
      <StatCard title="Temperatures" health={tempsHealth}>
        <ul class="temps">
          {#each system.temps as temp (temp.label)}
            <li data-health={temp.health}>
              <span class="temp-label">{temp.label}</span>
              <span class="temp-value">{formatValue('celsius', temp.celsius)}</span>
              <HealthLabel health={temp.health} size={14} hideText />
            </li>
          {/each}
        </ul>
      </StatCard>
    {/if}
  {:else}
    <StatCard title="Metrics" health={system.health} wide>
      <p class="value muted">No data yet</p>
      <p class="detail">Waiting for the first system sample.</p>
    </StatCard>
  {/if}

  <StatCard title="Host" wide>
    <p class="value">
      <span class="up-label">up</span>
      <span title="Booted {formatTimestamp(hostInfo.bootTime)}">{formatDuration(uptime)}</span>
    </p>
    <dl class="facts">
      <div>
        <dt>OS</dt>
        <dd>{hostInfo.os}</dd>
      </div>
      <div>
        <dt>Kernel</dt>
        <dd>{hostInfo.kernel}</dd>
      </div>
      <div>
        <dt>CPU</dt>
        <dd>{hostInfo.cpuModel} × {hostInfo.cores}</dd>
      </div>
    </dl>
  </StatCard>
</div>

<style>
  .grid {
    display: grid;
    /* min() keeps a single column from overflowing on very narrow screens. */
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 14rem), 1fr));
    gap: var(--space-3);
  }

  .value {
    font-size: var(--text-2xl);
    font-weight: var(--weight-semibold);
    line-height: var(--leading-tight);
    font-variant-numeric: tabular-nums;
  }

  .muted {
    color: var(--color-text-faint);
  }

  .up-label {
    font-size: var(--text-md);
    font-weight: var(--weight-regular);
    color: var(--color-text-muted);
  }

  .detail {
    margin-top: auto;
    font-size: var(--text-sm);
    color: var(--color-text-muted);
    font-variant-numeric: tabular-nums;
  }

  dl {
    margin: 0;
  }

  dd {
    margin: 0;
  }

  .load {
    display: flex;
    align-items: baseline;
    gap: var(--space-4);
    font-variant-numeric: tabular-nums;
  }

  .load div {
    display: flex;
    flex-direction: column-reverse;
  }

  .load dt {
    font-size: var(--text-xs);
    color: var(--color-text-faint);
  }

  .load dd:not(.value) {
    font-size: var(--text-lg);
    color: var(--color-text-muted);
  }

  .rates {
    display: grid;
    gap: var(--space-1);
    font-variant-numeric: tabular-nums;
  }

  .rates div {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .rates dt {
    font-size: var(--text-xs);
    color: var(--color-text-faint);
  }

  .rates dd:not(.value) {
    font-size: var(--text-lg);
    color: var(--color-text-muted);
  }

  .rates .value {
    font-size: var(--text-xl);
  }

  .temps {
    display: grid;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--text-sm);
  }

  .temps li {
    display: grid;
    grid-template-columns: 1fr auto auto;
    align-items: center;
    gap: var(--space-2);
  }

  .temp-label {
    min-width: 0;
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }

  .temp-value {
    font-variant-numeric: tabular-nums;
  }

  .facts {
    display: grid;
    gap: var(--space-1);
    font-size: var(--text-sm);
  }

  .facts div {
    display: grid;
    grid-template-columns: 4rem 1fr;
    gap: var(--space-2);
  }

  .facts dt {
    color: var(--color-text-faint);
  }

  .facts dd {
    overflow-wrap: anywhere;
  }
</style>
