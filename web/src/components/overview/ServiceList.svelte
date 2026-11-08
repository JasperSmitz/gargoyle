<script lang="ts">
  import type { ServiceSummary } from '../../lib/api/types';
  import { formatBytes, formatDuration, formatTimestamp } from '../../lib/format';
  import { sortBySeverity } from '../../lib/health';
  import CardList from '../CardList.svelte';
  import EmptyState from '../EmptyState.svelte';
  import Section from '../Section.svelte';
  import TargetRow from '../TargetRow.svelte';

  let { services, serverNow }: { services: ServiceSummary[]; serverNow: number } = $props();

  const sorted = $derived(sortBySeverity(services));
</script>

<Section title="Services" count={services.length}>
  {#if services.length === 0}
    <EmptyState title="No services configured">
      Add <code>[[service]]</code> entries to <code>gargoyle.toml</code> to watch systemd units; see
      <code>config/gargoyle.example.toml</code>.
    </EmptyState>
  {:else}
    <CardList>
      {#each sorted as service (service.unit)}
        <TargetRow
          name={service.name}
          id={service.unit}
          health={service.health}
          message={service.message}
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
  {/if}
</Section>
