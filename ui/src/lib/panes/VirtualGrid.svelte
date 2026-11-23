<script lang="ts">
  import { type Snippet, untrack } from 'svelte';

  import { fits, type Range, reveal, span, windowOf } from './scroll';

  /**
   * The same list as {@link VirtualRows}, laid out as tiles: item 0 is at the top left and the items run along a row of
   * tiles, then along the next one. A row holds as many tiles as fit the list's width (they share it), and the list
   * scrolls down. Only the rows of tiles in view (plus `overscan` above and below) are rendered.
   */
  let {
    count,
    tileWidth = 136,
    tileHeight = 172,
    overscan = 1,
    active,
    pageSize = $bindable(1),
    perRow = $bindable(1),
    visible = $bindable({ first: 0, last: 0 }),
    near = $bindable({ first: 0, last: 0 }),
    tile,
  }: {
    count: number;
    /** The narrowest a tile gets; a row has as many of them as fit, and they share its width. */
    tileWidth?: number;
    tileHeight?: number;
    /** Rows of tiles rendered beyond the ones in view, on each side. */
    overscan?: number;
    /** The item to keep in view (the cursor); the list scrolls only when it is out of view. */
    active?: number;
    /** The items that fit the list at once: the step of the Page keys. */
    pageSize?: number;
    /** The tiles in a row: the step of Up and Down. */
    perRow?: number;
    /** The items in view, and those rendered (a little beyond), for what is loaded as they come into view. */
    visible?: Range;
    near?: Range;
    /** Renders item `index`. */
    tile: Snippet<[number]>;
  } = $props();

  let scrollTop = $state(0);
  let width = $state(0);
  let height = $state(0);
  let viewport = $state<HTMLElement>();

  const across = $derived(fits(width, tileWidth));
  const cellWidth = $derived(width > 0 ? width / across : tileWidth);
  // Not laid out yet (no width or no height): no rows, rather than one per item, whose spacer would be
  // millions of pixels high.
  // A pane shorter than a tile still shows one row (cut), and the keys move by it.
  const rows = $derived(width <= 0 || height <= 0 ? 0 : Math.ceil(count / across));
  const inView = $derived(windowOf(scrollTop, height, tileHeight, 0, rows));
  const rendered = $derived(windowOf(scrollTop, height, tileHeight, overscan, rows));
  const first = $derived(rendered.first * across);
  const last = $derived(Math.min(count, rendered.last * across));

  $effect(() => {
    pageSize = fits(height, tileHeight) * across;
    perRow = across;
  });

  $effect(() => {
    visible = { first: inView.first * across, last: Math.min(count, inView.last * across) };
    near = { first, last };
  });

  // Scrolls when `active` moves (or the tiles are laid out anew), not when the user scrolls.
  $effect(() => {
    if (active === undefined || active < 0 || viewport === undefined) {
      return;
    }

    const top = Math.floor(active / across) * tileHeight;
    const bottom = top + tileHeight;
    untrack(() => {
      scrollTop = reveal(viewport, 'y', top, bottom) ?? scrollTop;
    });
  });

  const indices = $derived(span(first, last));
  const windowStyle = $derived(
    `transform: translateY(${rendered.first * tileHeight}px); grid-template-columns: repeat(${across}, ${cellWidth}px); grid-auto-rows: ${tileHeight}px`,
  );
</script>

<div
  class="viewport"
  role="presentation"
  bind:this={viewport}
  bind:clientWidth={width}
  bind:clientHeight={height}
  onscroll={(event) => (scrollTop = event.currentTarget.scrollTop)}
>
  <div class="spacer" style:height="{rows * tileHeight}px">
    <div class="window" style={windowStyle}>
      {#each indices as index (index)}
        {@render tile(index)}
      {/each}
    </div>
  </div>
</div>

<style>
  .viewport {
    height: 100%;
    overflow: hidden auto;
    contain: strict;
  }

  .spacer {
    position: relative;
  }

  /* Items fill a row of tiles from the left, then the next one. */
  .window {
    position: absolute;
    inset: 0 0 auto;
    display: grid;
    will-change: transform;
  }
</style>
