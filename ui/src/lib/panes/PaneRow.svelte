<script lang="ts">
  import type { Entry } from '$lib/ipc/bindings/Entry';

  import { displayName, formatDate, iconClass, splitName } from './format';
  import type { InlineRename } from './inlineRename.svelte';
  import RenameField from './RenameField.svelte';

  let {
    id,
    index,
    entry,
    up = false,
    cursor,
    selected,
    size,
    edit,
  }: {
    id: string;
    /** The row number, for the list that handles the mouse for all its rows. */
    index: number;
    /** `undefined` for `..` and for an entry whose batch has not arrived yet. */
    entry: Entry | undefined;
    up?: boolean;
    cursor: boolean;
    selected: boolean;
    /** The Size column; `undefined` for a folder not measured, which shows the DIR tag. */
    size: string | undefined;
    /** Set on the row being renamed in place: its name is a field then. */
    edit?: InlineRename;
  } = $props();

  const parts = $derived(entry ? splitName(entry) : undefined);
</script>

<div
  {id}
  class="row"
  class:cursor
  class:selected
  class:d={entry?.kind === 'dir'}
  class:h={entry?.hidden}
  role="option"
  data-row={index}
  aria-selected={selected}
>
  {#if up}
    <span class="ic up">↰</span><span class="nm">..</span>
  {:else if entry && parts}
    <span class="ic {iconClass(entry)}"></span>
    {#if edit}
      <RenameField rename={edit} />
    {:else}
      <span class="nm" title={entry.linkTarget ? displayName(`${entry.name} → ${entry.linkTarget}`) : undefined}>{displayName(parts.base)}</span>
      <span class="ex">{displayName(parts.ext)}</span>
    {/if}
    <span class="sz">{#if size !== undefined}{size}{:else}<span class="dirtag">DIR</span>{/if}</span>
    <span class="dt">{formatDate(entry.modified)}</span>
  {/if}
</div>

<style>
  .row {
    display: grid;
    grid-template-columns: var(--pane-cols);
    align-items: center;
    gap: 8px;
    height: 24px;
    padding: 0 10px;
    border-left: 2px solid transparent;
    cursor: default;
    user-select: none;
  }

  .row:hover {
    background: color-mix(in srgb, var(--text) 4%, transparent);
  }

  .row.cursor {
    border-left-color: var(--cursor-line);
    background: var(--cursor-bg);
  }

  .row.selected :is(.nm, .ex, .sz, .dt) {
    color: var(--accent);
  }

  .row.d .nm {
    font-weight: 600;
  }

  .row.h {
    opacity: 0.6;
  }

  .nm,
  .ex,
  .dt {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ex,
  .dt {
    font-size: 12px;
    color: var(--dim);
  }

  .sz {
    font: 11.5px var(--font-mono);
    color: var(--dim);
    text-align: right;
    white-space: nowrap;
  }

  .dirtag {
    font-size: 10px;
    letter-spacing: 0.06em;
    opacity: 0.7;
  }

  .ic {
    position: relative;
    display: inline-block;
    flex: none;
    width: 16px;
    height: 12px;
  }

  .ic.dir {
    margin-top: 2px;
    border-radius: 1px 3px 3px;
    background: var(--dir);
  }

  .ic.dir::before {
    content: '';
    position: absolute;
    top: -3px;
    left: 0;
    width: 7px;
    height: 4px;
    border-radius: 2px 2px 0 0;
    background: var(--dir);
  }

  .ic.up {
    color: var(--dim);
    font-size: 14px;
    line-height: 12px;
    text-align: center;
  }

  .ic.f,
  .ic.img,
  .ic.code,
  .ic.arc,
  .ic.doc,
  .ic.exe {
    width: 12px;
    height: 15px;
    margin-left: 2px;
    border-radius: 2px;
    background: var(--fc, #8a93a8);
    clip-path: polygon(0 0, 66% 0, 100% 30%, 100% 100%, 0 100%);
  }

  .ic.img {
    --fc: #b98bff;
  }

  .ic.code {
    --fc: #ff8a5b;
  }

  .ic.arc {
    --fc: #e0b44c;
  }

  .ic.doc {
    --fc: #6aa8ff;
  }

  .ic.exe {
    --fc: #4cc38a;
  }

  @media (max-width: 760px) {
    .ex,
    .dt {
      display: none;
    }
  }
</style>
