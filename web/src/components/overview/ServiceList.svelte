<script lang="ts">
  import type { ServiceSummary } from '../../lib/api/types';
  import { formatBytes, formatDuration, formatTimestamp } from '../../lib/format';
  import { sortBySeverity } from '../../lib/health';
  import { href } from '../../lib/router';
  import CardList from '../CardList.svelte';
  import Section from '../Section.svelte';
  import TargetRow from '../TargetRow.svelte';

  // Only rendered with at least one service; the overview explains an empty config instead.
  let { services, serverNow }: { services: ServiceSummary[]; serverNow: number } = $props();

  const sorted = $derived(sortBySeverity(services));
</script>

<Section title="Services" count={services.length}>
  <CardList>
    {#each sorted as service (service.unit)}
      <TargetRow
        name={service.name}
        id={service.unit}
        health={service.health}
        message={service.message}
        href={href('services', service.unit)}
      >
        {#snippet meta()}
          <span>{service.activeState} ({service.subState})</span>
          {#if service.since !== null}
            <span title="Since {formatTimestamp(service.since)}">
              for {formatDuration(serverNow - service.since)}
            </span>
          {/if}
          {#if service.memory !== null}<span>{formatBytes(service.memory)}</span>{/if}
          {#if service.restarts > 0}
            <span>{service.restarts} {service.restarts === 1 ? 'restart' : 'restarts'}</span>
          {/if}
        {/snippet}
      </TargetRow>
    {/each}
  </CardList>
</Section>
