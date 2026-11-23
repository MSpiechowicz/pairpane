<script lang="ts">
  import { type Snippet, untrack } from 'svelte';

  import { fits, reveal, span, windowOf } from './scroll';

  /**
   * The same list as {@link VirtualRows}, laid out in columns: item 0 is at the top of the first column and the items
   * run down it, then down the next one. A column is as high as the list and as wide as its share of the list's
   * width (a whole number of columns fits); further columns scroll in sideways. Only the columns in view (plus
   * `overscan` on each side) are rendered.
   */
  let {
    count,
    rowHeight = 24,
    minColumnWidth = 180,
    overscan = 2,
    active,
    pageSize = $bindable(1),
    row,
  }: {
    count: number;
    rowHeight?: number;
    /** The narrowest a column gets; the list shows as many of them as fit and shares its width among them. */
    minColumnWidth?: number;
    /** Columns rendered beyond the ones in view, on each side. */
    overscan?: number;
    /** The item to keep in view (the cursor); the list scrolls only when it is out of view. */
    active?: number;
    /** The items in a column: the step of the Page keys, and of Left and Right. */
    pageSize?: number;
    /** Renders item `index`. */
    row: Snippet<[number]>;
  } = $props();

  let scrollLeft = $state(0);
  let width = $state(0);
  let height = $state(0);
  let viewport = $state<HTMLElement>();

  const rows = $derived(fits(height, rowHeight));
  const columnWidth = $derived(width > 0 ? width / fits(width, minColumnWidth) : minColumnWidth);
  // Not laid out yet (no height, or under one row): no columns, rather than one per item, whose spacer would be
  // millions of pixels wide.
  const columns = $derived(height < rowHeight ? 0 : Math.ceil(count / rows));
  const shown = $derived(windowOf(scrollLeft, width, columnWidth, overscan, columns));
  const firstColumn = $derived(shown.first);
  const first = $derived(firstColumn * rows);
  const last = $derived(Math.min(count, shown.last * rows));
  $effect(() => {
    pageSize = rows;
  });

  // Scrolls when `active` moves (or the columns are laid out anew), not when the user scrolls.
  $effect(() => {
    if (active === undefined || active < 0 || viewport === undefined) {
      return;
    }

    const left = Math.floor(active / rows) * columnWidth;
    const right = left + columnWidth;
    untrack(() => {
      scrollLeft = reveal(viewport, 'x', left, right) ?? scrollLeft;
    });
  });

  const indices = $derived(span(first, last));
</script>

<div
  class="viewport"
  role="presentation"
  bind:this={viewport}
  bind:clientWidth={width}
  bind:clientHeight={height}
  onscroll={(event) => (scrollLeft = event.currentTarget.scrollLeft)}
>
  <div class="spacer" style:width="{columns * columnWidth}px" style:height="{rows * rowHeight}px">
    <div
      class="window"
      style:transform="translateX({firstColumn * columnWidth}px)"
      style:grid-template-rows="repeat({rows}, {rowHeight}px)"
      style:grid-auto-columns="{columnWidth}px"
    >
      {#each indices as index (index)}
        {@render row(index)}
      {/each}
    </div>
  </div>
</div>

<style>
  .viewport {
    height: 100%;
    overflow: auto hidden;
    contain: strict;
  }

  .spacer {
    position: relative;
  }

  /* Items fill a column from the top, then the next one. */
  .window {
    position: absolute;
    inset: 0 auto auto 0;
    display: grid;
    grid-auto-flow: column;
    will-change: transform;
  }
</style>
