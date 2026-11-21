<script lang="ts">
  import { positionAt, scrollRange, scrollTopAt } from './scroll';

  let {
    extent,
    position,
    visible = true,
    onSeek,
  }: {
    /** The largest position: the end of what is scrolled, in bytes. */
    extent: number;
    /** Where the view is (`0..=extent`); the thumb follows it. */
    position: number;
    /** A file that fits the view has nothing to scroll. */
    visible?: boolean;
    /** The user moved the thumb: the position it stands for. */
    onSeek: (position: number) => void;
  } = $props();

  /** How long after the user's last move the thumb is left alone (it would fight the drag by snapping to a line). */
  const SETTLE_MS = 250;

  let track = $state<HTMLElement>();
  let height = $state(0);
  const range = $derived(scrollRange(extent, height));
  /** The scroll top set from the position, to tell the scroll event it causes from the user's own. */
  let applied = 0;
  let dragging = false;
  let settle: ReturnType<typeof setTimeout> | undefined;

  $effect(() => {
    const top = scrollTopAt(position, extent, range);
    if (track !== undefined && !dragging && Math.abs(track.scrollTop - top) >= 1) {
      applied = top;
      track.scrollTop = top;
    }
  });

  $effect(() => () => clearTimeout(settle));

  function onscroll() {
    if (track === undefined || Math.abs(track.scrollTop - applied) < 1) {
      return;
    }

    dragging = true;
    clearTimeout(settle);
    settle = setTimeout(() => (dragging = false), SETTLE_MS);
    onSeek(positionAt(track.scrollTop, extent, range));
  }
</script>

<div class="track" class:hidden={!visible} bind:this={track} bind:clientHeight={height} {onscroll} tabindex="-1" aria-hidden="true">
  <div style:height="{height + range}px"></div>
</div>

<style>
  .track {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: 16px;
    overflow: hidden scroll;
  }

  .hidden {
    visibility: hidden;
  }
</style>
