<script lang="ts">
  import { type Snippet, untrack } from 'svelte';

  import { fits, type Range, reveal, span, windowOf } from './scroll';

  /**
   * A scrolling list of `count` fixed-height rows that renders only the rows in view (plus `overscan` above and
   * below), so a folder of 100 000 entries costs as much as one screen.
   */
  let {
    count,
    rowHeight = 24,
    overscan = 12,
    active,
    pageSize = $bindable(1),
    visible = $bindable({ first: 0, last: 0 }),
    near = $bindable({ first: 0, last: 0 }),
    row,
  }: {
    count: number;
    rowHeight?: number;
    overscan?: number;
    /** The row to keep in view (the cursor); the list scrolls only when it is out of view. */
    active?: number;
    /** How many whole rows fit in the list. */
    pageSize?: number;
    /** The rows in view, and those rendered (a little beyond), for what is loaded as they come into view. */
    visible?: Range;
    near?: Range;
    /** Renders row `index`. */
    row: Snippet<[number]>;
  } = $props();

  let scrollTop = $state(0);
  let height = $state(0);
  let viewport = $state<HTMLElement>();

  const shown = $derived(windowOf(scrollTop, height, rowHeight, overscan, count));
  const inView = $derived(windowOf(scrollTop, height, rowHeight, 0, count));
  $effect(() => {
    pageSize = fits(height, rowHeight);
  });

  $effect(() => {
    visible = inView;
    near = shown;
  });

  // Scrolls when `active` changes, not when the user scrolls: only `active` is tracked.
  $effect(() => {
    if (active === undefined || active < 0 || viewport === undefined) {
      return;
    }

    const top = active * rowHeight;
    const bottom = top + rowHeight;
    untrack(() => {
      scrollTop = reveal(viewport, 'y', top, bottom) ?? scrollTop;
    });
  });

  const indices = $derived(span(shown.first, shown.last));
</script>

<div
  class="viewport"
  role="presentation"
  bind:this={viewport}
  bind:clientHeight={height}
  onscroll={(event) => (scrollTop = event.currentTarget.scrollTop)}
>
  <div class="spacer" style:height="{count * rowHeight}px">
    <div class="window" style:transform="translateY({shown.first * rowHeight}px)">
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
