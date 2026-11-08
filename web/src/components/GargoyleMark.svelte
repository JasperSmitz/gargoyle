<script lang="ts">
  // Horned stone shield with verdigris eyes; same shape as the favicon in index.html.
  let { size = 22 }: { size?: number } = $props();
</script>

<svg
  class="mark"
  viewBox="0 0 32 32"
  width={size}
  height={size}
  aria-hidden="true"
  focusable="false"
>
  <path class="stone" d="M6 3l4.5 5h11L26 3v13c0 7-4.6 11.2-10 13.5C10.6 27.2 6 23 6 16z" />
  <circle class="eye" cx="12.5" cy="15.5" r="2" />
  <circle class="eye" cx="19.5" cy="15.5" r="2" />
</svg>

<style>
  .mark {
    flex: none;
  }

  .stone {
    fill: var(--color-text-faint);
  }

  .eye {
    fill: var(--color-accent);
  }
</style>
