<script lang="ts">
  import type { Snippet } from 'svelte';

  /**
   * A scrolling list of `count` fixed-height rows that renders only the rows in view (plus `overscan` above and
   * below), so a folder of 100 000 entries costs as much as one screen.
   */
  let {
    count,
    rowHeight = 24,
    overscan = 12,
    row,
  }: {
    count: number;
    rowHeight?: number;
    overscan?: number;
    /** Renders row `index`. */
    row: Snippet<[number]>;
  } = $props();

  let scrollTop = $state(0);
  let height = $state(0);

  const first = $derived(Math.max(0, Math.floor(scrollTop / rowHeight) - overscan));
  const last = $derived(Math.min(count, Math.ceil((scrollTop + height) / rowHeight) + overscan));
  const indices = $derived(Array.from({ length: Math.max(0, last - first) }, (_, offset) => first + offset));
</script>

<div
  class="viewport"
  role="presentation"
  bind:clientHeight={height}
  onscroll={(event) => (scrollTop = event.currentTarget.scrollTop)}
>
  <div class="spacer" style:height="{count * rowHeight}px">
    <div class="window" style:transform="translateY({first * rowHeight}px)">
      {#each indices as index (index)}
        {@render row(index)}
      {/each}
    </div>
  </div>
</div>

<style>
  .viewport {
    height: 100%;
    overflow: auto;
    contain: strict;
  }

  .spacer {
    position: relative;
  }

  .window {
    position: absolute;
    inset: 0 0 auto;
    will-change: transform;
  }
</style>
