<script lang="ts">
  import { formatTimestamp, isTimestamp, PLACEHOLDER } from '../lib/format';

  // Every point in time on screen goes through here: machine-readable `datetime`, and when the
  // visible text is relative ("3h ago", "in 5m", "for 2d") the absolute local time as a tooltip.
  let {
    ts,
    text,
    title,
  }: {
    /** Unix seconds; `0`/null render the placeholder. */
    ts: number | null;
    /** Visible text; defaults to the absolute time (then no tooltip is needed). */
    text?: string;
    /** Tooltip override, e.g. "Booted …"; defaults to the absolute time. */
    title?: string;
  } = $props();

  const absolute = $derived(formatTimestamp(ts));
</script>

{#if isTimestamp(ts)}
  <time
    datetime={new Date(ts * 1000).toISOString()}
    title={title ?? (text === undefined ? undefined : absolute)}>{text ?? absolute}</time
  >
{:else}
  <span>{text ?? PLACEHOLDER}</span>
{/if}
