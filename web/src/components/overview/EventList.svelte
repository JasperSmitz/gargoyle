<script lang="ts">
  import type { HealthEvent } from '../../lib/api/types';
  import { href } from '../../lib/router';
  import CardList from '../CardList.svelte';
  import EventItem from '../events/EventItem.svelte';
  import Section from '../Section.svelte';

  // The overview's short list; the full log is the events page.
  let { events, serverNow }: { events: HealthEvent[]; serverNow: number } = $props();
</script>

<Section title="Recent events">
  <CardList>
    {#each events as event (event.id)}
      <EventItem {event} {serverNow} />
    {/each}
  </CardList>
  <a class="more" href={href('events')}>All events</a>
</Section>

<style>
  .more {
    align-self: flex-start;
    font-size: var(--text-sm);
  }
</style>
