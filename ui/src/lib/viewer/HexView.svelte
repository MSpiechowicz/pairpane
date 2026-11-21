<script lang="ts">
  import { untrack } from 'svelte';

  import { hexRow, offsetDigits, ROW_BYTES } from './hex';
  import { HexModel, ROW_HEIGHT } from './hexWindow.svelte';
  import ScrollTrack from './ScrollTrack.svelte';
  import type { ViewSession } from './session.svelte';
  import ViewNotice from './ViewNotice.svelte';
  import { onNewHit, takeFocus, type ViewProps } from './viewProps.svelte';

  let { session, hit, focus = false }: ViewProps = $props();

  const model = new HexModel(untrack(() => session));
  let height = $state(0);

  const digits = $derived(offsetDigits(session.size));
  const rows = $derived.by(() => {
    const offsets = Array.from({ length: model.rows + 1 }, (_, index) => model.top + index * ROW_BYTES);

    // Only the rows whose bytes are held: those that were just scrolled to appear when their bytes arrive.
    return offsets
      .filter((offset) => offset < session.size && offset >= model.base && offset < model.base + model.bytes.length)
      .map((offset) => hexRow(offset, model.bytes, model.base, digits, hit));
  });

  // Starts the view at the position the text view left, and again when the rows that fit change.
  $effect(() => {
    if (height > 0) {
      model.rows = Math.max(1, Math.floor(height / ROW_HEIGHT));
    }

    untrack(() => void model.moveTo(session.position));
  });

  onNewHit(
    () => hit,
    (offset) => void model.moveTo(offset),
  );

  function onkeydown(event: KeyboardEvent) {
    if (model.key(event)) {
      event.preventDefault();
    }
  }
</script>

<div class="hex">
  <div
    class="viewport"
    role="textbox"
    aria-readonly="true"
    aria-multiline="true"
    aria-label="Bytes of {session.name}"
    tabindex="0"
    data-autofocus={focus ? '' : undefined}
    bind:clientHeight={height}
    {onkeydown}
    onwheel={(event) => model.wheel(event)}
    {@attach takeFocus(focus)}
  >
    {#each rows as row (row.offset)}
      <div class="row">
        <span class="offset">{row.label}</span>
        <span class="bytes">
          {#each row.cells as cell, at (at)}<span class:hit={cell.hit} aria-current={cell.hit ? 'true' : undefined}>{cell.hex}</span>{' '}{/each}
        </span>
        <span class="ascii">{#each row.cells as cell, at (at)}<span class:hit={cell.hit}>{cell.char}</span>{/each}</span>
      </div>
    {/each}
  </div>
  <ViewNotice error={model.error} text={model.ready && session.size === 0 ? 'The file is empty.' : undefined} />
  <ScrollTrack
    extent={model.limit}
    position={model.top}
    visible={model.limit > 0}
    onSeek={(position) => void model.moveTo(position)}
  />
</div>

<style>
  .hex {
    position: relative;
    flex: 1;
    min-height: 0;
  }

  .viewport {
    height: 100%;
    padding: 4px 22px 4px 12px;
    overflow: hidden;
    outline: none;
    font: 12.5px/18px var(--font-mono);
  }

  .row {
    display: flex;
    gap: 16px;
    height: 18px;
    white-space: pre;
  }

  .offset {
    color: var(--dim);
  }

  .bytes {
    display: grid;
    grid-template-columns: repeat(16, 2ch);
    column-gap: 1ch;
    width: 47ch;
  }

  .ascii {
    color: var(--dim);
  }

  .hit {
    background: color-mix(in srgb, var(--accent) 35%, transparent);
    color: var(--text);
  }
</style>
