<script lang="ts">
  import { untrack } from 'svelte';

  import { displayName } from '$lib/panes/format';
  import { ROW_HEIGHT } from '$lib/viewer/hexWindow.svelte';
  import ViewNotice from '$lib/viewer/ViewNotice.svelte';
  import { takeFocus } from '$lib/viewer/viewProps.svelte';

  import DiffFrame from './DiffFrame.svelte';
  import DiffNote from './DiffNote.svelte';
  import DiffToolbar from './DiffToolbar.svelte';
  import type { FileDiff } from './fileDiff.svelte';
  import { hexOffset, sizeLine } from './hexDiff';
  import type { HexDiff } from './hexDiff.svelte';

  let { diff, hex }: { diff: FileDiff; hex: HexDiff } = $props();

  let height = $state(0);

  const sizes = $derived(sizeLine(hex.left.size, hex.right.size, hex.first));

  const summary = $derived.by(() => {
    if (hex.searching !== undefined) {
      return `Searching… ${hexOffset(hex.searching)}`;
    }

    return hex.message ?? 'Bytes';
  });

  // Lays the columns out for the rows that fit, and reads what they show; the first difference is looked for once both files are open.
  $effect(() => {
    if (height > 0) {
      hex.setRows(Math.max(1, Math.floor(height / ROW_HEIGHT)));
    }

    if (hex.status === 'ready') {
      untrack(() => {
        hex.begin();
        void hex.lead.moveTo(hex.top);
      });
    }
  });

  /** F7 and Shift+F7 move between the differences. */
  function onwindowkeydown(event: KeyboardEvent) {
    if (event.key === 'F7' && !event.isComposing && !event.ctrlKey && !event.altKey && !event.metaKey && !event.defaultPrevented) {
      event.preventDefault();
      if (event.shiftKey) {
        hex.previous();
      } else {
        hex.next();
      }
    }
  }

  function onkeydown(event: KeyboardEvent) {
    if (hex.lead.key(event)) {
      event.preventDefault();
    }
  }
</script>

<svelte:window onkeydown={onwindowkeydown} />

<DiffFrame>
  <DiffToolbar {summary} disabled={hex.status !== 'ready'} onPrevious={() => hex.previous()} onNext={() => hex.next()} onClose={() => void diff.requestClose()} />

  <DiffNote
    role="note"
    text="One of the files is not text the editor can open (or is larger than it takes), so the two are compared byte by byte and cannot be changed here."
  />

  {#if hex.status === 'failed'}
    <DiffNote role="alert" text={hex.failure ?? 'The files could not be opened.'} />
  {:else if hex.status === 'ready'}
    <DiffNote text={sizes} />

    <div class="names">
      <span class="gap"></span>
      <span class="name">{displayName(hex.left.name)}</span>
      <span class="name">{displayName(hex.right.name)}</span>
    </div>

    <div
      class="viewport"
      role="textbox"
      aria-readonly="true"
      aria-multiline="true"
      aria-label="Bytes of {displayName(hex.left.name)} and {displayName(hex.right.name)}"
      tabindex="0"
      data-autofocus
      bind:clientHeight={height}
      {onkeydown}
      onwheel={(event) => hex.lead.wheel(event)}
      {@attach takeFocus(true)}
    >
      {#each hex.lines as row (row.offset)}
        <div class="row">
          <span class="offset">{row.label}</span>
          {#each [row.left, row.right] as cells, column (column)}
            <span class="bytes">
              {#each cells as cell, at (at)}<span class:differs={cell.differs} class:blank={cell.blank} class:current={cell.current} aria-current={cell.current ? 'true' : undefined}
                  >{cell.hex}</span
                >{/each}
            </span>
          {/each}
        </div>
      {/each}
    </div>
    <ViewNotice error={hex.lead.error ?? hex.follower.error} text={undefined} />
  {/if}
</DiffFrame>

<style>
  .names,
  .row {
    display: grid;
    grid-template-columns: 8ch 1fr 1fr;
    column-gap: 16px;
  }

  .names {
    padding: 4px 12px;
    border-bottom: 1px solid var(--line);
    background: var(--panel);
    font-weight: 600;
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .viewport {
    flex: 1;
    min-height: 0;
    padding: 4px 12px;
    overflow: hidden;
    outline: none;
    font: 12.5px/18px var(--font-mono);
  }

  .row {
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
  }

  .differs {
    background: color-mix(in srgb, var(--danger) 30%, transparent);
  }

  .current {
    outline: 1px solid var(--sel);
  }

  /* A byte the shorter file does not have: hatched, so that it is not mistaken for a zero. */
  .blank {
    background: repeating-linear-gradient(135deg, transparent 0 3px, color-mix(in srgb, var(--dim) 35%, transparent) 3px 4px);
  }
</style>
