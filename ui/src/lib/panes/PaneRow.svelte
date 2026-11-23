<script lang="ts">
  import { type Mark, MARK_TEXT } from '$lib/compare/quick';
  import type { Entry } from '$lib/ipc/bindings/Entry';
  import { preferences } from '$lib/shell/preferences.svelte';
  import { type Decoded, drawDecoded } from '$lib/viewer/decoded';

  import type { Cell } from './extras';
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
    mark,
    brief = false,
    tile = false,
    thumb,
    extra,
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
    /** How the entry stands against the other pane's, after a comparison: a tint, and the same in words. */
    mark?: Mark;
    /** The brief layout: the icon and the name alone, the extension written after the name, no other column. */
    brief?: boolean;
    /** The thumbnails layout: a tile with the picture (or the icon) above the name, no other column. */
    tile?: boolean;
    /** The thumbnail to show in a tile in place of the icon. */
    thumb?: Decoded;
    /** The Dimensions, Checksum and Git columns, as far as the settings show them; empty until the value arrives. */
    extra?: { dimensions: Cell; checksum: Cell; git: Cell };
  } = $props();

  const parts = $derived(entry ? splitName(entry) : undefined);
  /** The brief layout and the tiles show nothing but the name (and its picture). */
  const plain = $derived(brief || tile);
  /** The columns the row has besides the name; the brief and thumbnails layouts have none. */
  const columns = $derived({
    ext: !plain && preferences.value.showExt,
    size: !plain && preferences.value.showSize,
    modified: !plain && preferences.value.showModified,
    dimensions: !plain && preferences.value.showDimensions,
    checksum: !plain && preferences.value.showChecksum,
    git: !plain && preferences.value.showGit,
  });

  /** The tooltip of a name: where a link points; in the brief and thumbnails layouts the whole name, which may be cut short. */
  function tip(of: Entry): string | undefined {
    if (of.linkTarget) {
      return displayName(`${of.name} → ${of.linkTarget}`);
    }

    return plain ? displayName(of.name) : undefined;
  }
</script>

<div
  {id}
  class="row {mark ? `mark-${mark}` : ''}"
  class:brief
  class:tile
  class:cursor
  class:selected
  class:d={entry?.kind === 'dir'}
  class:h={entry?.hidden}
  title={mark && MARK_TEXT[mark]}
  aria-describedby={mark ? `${id}-mark` : undefined}
  role="option"
  data-row={index}
  aria-selected={selected}
>
  {#if mark}
    <span id="{id}-mark" class="sr-only" aria-hidden="true">{MARK_TEXT[mark]}</span>
  {/if}
  {#if up}
    <span class="ic up">↰</span><span class="nm">..</span>
  {:else if entry && parts}
    {#if tile && thumb}
      {@const picture = thumb}
      <!-- The name is under the picture, which only repeats it. -->
      <canvas
        class="pic"
        width={picture.width}
        height={picture.height}
        aria-hidden="true"
        {@attach (canvas) => void drawDecoded(canvas, picture)}
      ></canvas>
    {:else}
      <span class="ic {iconClass(entry)}"></span>
    {/if}
    {#if edit}
      <RenameField rename={edit} />
    {:else}
      <!-- The extension is written after the name wherever its own column is not on the screen (hidden by the settings,
        or the narrow layout), so that report.pdf and report.png never read as the same name. While the column is there, it
        is the one a screen reader reads. -->
      <span class="nm" title={tip(entry)}
        >{displayName(parts.base)}{#if parts.ext}<span class="nx" class:shown={!columns.ext} aria-hidden={columns.ext}
            >.{displayName(parts.ext)}</span
          >{/if}</span
      >
      {#if columns.ext}
        <span class="ex">{displayName(parts.ext)}</span>
      {/if}
    {/if}
    {#if columns.size}
      <span class="sz">{#if size !== undefined}{size}{:else}<span class="dirtag">DIR</span>{/if}</span>
    {/if}
    {#if columns.modified}
      <span class="dt">{formatDate(entry.modified)}</span>
    {/if}
    {#if columns.dimensions}
      <span class="dm" title={extra?.dimensions.tip}>{extra?.dimensions.text}</span>
    {/if}
    {#if columns.checksum}
      <span class="ck" title={extra?.checksum.tip}>{extra?.checksum.text}</span>
    {/if}
    {#if columns.git}
      <span class="gt {extra?.git.tone ?? ''}" title={extra?.git.tip}>{extra?.git.text}</span>
    {/if}
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

  /* The icon and the name alone. */
  .row.brief {
    --rename-end: 3;
    grid-template-columns: 20px minmax(0, 1fr);
  }

  /* The picture or icon above the name, both centered; the name is cut to two lines. */
  .row.tile {
    --rename-start: 1;
    --rename-end: 2;
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: 128px auto;
    align-content: start;
    justify-items: center;
    gap: 4px;
    height: auto;
    padding: 2px;
    border: 2px solid transparent;
    border-radius: 6px;
  }

  .row.tile.cursor {
    border-color: var(--cursor-line);
  }

  /* The icons are drawn for a line of text: they are scaled up to fill the picture's place. */
  .row.tile .ic {
    align-self: center;
    transform: scale(3.5);
  }

  .row.tile .ic.up {
    transform: scale(4);
  }

  /* The picture is as large as the backend made it (it fits its size, perhaps a bucket above the one asked): it is scaled into the tile. */
  .pic {
    align-self: center;
    object-fit: contain;
    max-width: 100%;
    max-height: 128px;
  }

  .row.tile .nm {
    display: -webkit-box;
    max-width: 100%;
    line-clamp: 2;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    white-space: normal;
    overflow-wrap: anywhere;
    text-align: center;
  }

  .row:hover {
    background: color-mix(in srgb, var(--text) 4%, transparent);
  }

  .row.mark-onlyHere {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }

  .row.mark-newer {
    background: color-mix(in srgb, var(--ok) 14%, transparent);
  }

  .row.mark-older {
    background: color-mix(in srgb, var(--dim) 14%, transparent);
  }

  .row.mark-differs {
    background: color-mix(in srgb, var(--sel) 16%, transparent);
  }

  .row.cursor {
    border-left-color: var(--cursor-line);
    background: var(--cursor-bg);
  }

  .row.selected :is(.nm, .ex, .sz, .dt, .dm, .ck) {
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
  .dt,
  .dm,
  .ck,
  .gt {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ex,
  .dt,
  .dm {
    font-size: 12px;
    color: var(--dim);
  }

  .ck {
    font: 11.5px var(--font-mono);
    color: var(--dim);
  }

  /* The mark keeps its colour in a selected row: the colour is part of what it says. */
  .gt {
    font: 600 11.5px var(--font-mono);
    color: var(--dim);
  }

  .gt.accent {
    color: var(--accent);
  }

  .gt.ok {
    color: var(--ok);
  }

  .gt.danger {
    color: var(--danger);
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

  .nx {
    display: none;
  }

  .nx.shown {
    display: inline;
  }

  @media (max-width: 760px) {
    .ex,
    .dt,
    .dm,
    .ck,
    .gt {
      display: none;
    }

    .nx {
      display: inline;
    }
  }
</style>
