<script lang="ts">
  import { untrack } from 'svelte';

  import ColumnHeader from '$lib/panes/ColumnHeader.svelte';
  import { columnTracks, shows } from '$lib/panes/columns';
  import { extraTargetOf, wantedExtras } from '$lib/panes/extras';
  import { displayName } from '$lib/panes/format';
  import type { FlatSide } from '$lib/panes/flat.svelte';
  import type { Hotlist } from '$lib/panes/hotlist.svelte';
  import { handleKey } from '$lib/panes/keys';
  import type { InlineRename } from '$lib/panes/inlineRename.svelte';
  import PaneFooter from '$lib/panes/PaneFooter.svelte';
  import PaneRow from '$lib/panes/PaneRow.svelte';
  import type { Range } from '$lib/panes/scroll';
  import type { PaneTabs } from '$lib/panes/tabs.svelte';
  import { targetOf, wantedBy, whereIs } from '$lib/panes/thumbs';
  import type { Volumes } from '$lib/panes/volumes.svelte';
  import VirtualColumns from '$lib/panes/VirtualColumns.svelte';
  import VirtualGrid from '$lib/panes/VirtualGrid.svelte';
  import VirtualRows from '$lib/panes/VirtualRows.svelte';

  import DriveMenu from './DriveMenu.svelte';
  import HotlistMenu from './HotlistMenu.svelte';
  import type { Menu } from './menu';
  import PaneHeader from './PaneHeader.svelte';
  import { preferences } from './preferences.svelte';

  let {
    label,
    tabs,
    volumes,
    hotlist,
    menu,
    active,
    onActivate,
    onMenu,
    rename,
    flat,
  }: {
    /** Accessible name of the pane ("Left pane"). */
    label: string;
    tabs: PaneTabs;
    volumes: Volumes;
    hotlist: Hotlist;
    /** The dropdown shown under the header, if any. */
    menu: Menu | undefined;
    active: boolean;
    onActivate: () => void;
    onMenu: (menu: Menu | undefined) => void;
    /** The field for renaming a row in place; one for both panes. */
    rename?: InlineRename;
    /** This pane's flat view (every file below the folder), if the app has one. */
    flat?: FlatSide;
  } = $props();

  const tracks = $derived(columnTracks(preferences.value));
  const view = $derived(tabs.view);
  const brief = $derived(view.layout === 'brief');
  const thumbnails = $derived(view.layout === 'thumbnails');
  const detailed = $derived(view.layout === 'detailed');
  const thumbs = $derived(tabs.thumbs);
  const extras = $derived(tabs.extras);
  /** The extra columns the pane shows now: only the detailed layout has them. */
  const extraColumns = $derived({
    dimensions: detailed && preferences.value.showDimensions,
    checksum: detailed && preferences.value.showChecksum,
    git: detailed && preferences.value.showGit,
  });
  const listing = $derived(view.listing);
  const uid = $props.id();
  let list = $state<HTMLElement>();
  let pageSize = $state(1);
  /** The tiles in a row of the thumbnails layout, and the items in view and near it (what is worth a thumbnail). */
  let perRow = $state(1);
  let visible = $state<Range>({ first: 0, last: 0 });
  let near = $state<Range>({ first: 0, last: 0 });
  /** The row of this pane that is being renamed in place. */
  const editing = $derived(rename?.current?.view === view ? rename.current : undefined);

  // A rename is for the folder and tab it was started in.
  $effect(() => {
    if (editing !== undefined && (editing.path !== listing.path || editing.tab !== tabs.active)) {
      rename?.cancel();
    }
  });

  // Another folder: what was in view belonged to the last one. The grid tells the new ranges once it is laid out.
  $effect.pre(() => {
    void listing.location;
    visible = { first: 0, last: 0 };
    near = { first: 0, last: 0 };
  });

  // The thumbnails to fetch are those of the pictures in view, then those near it; none when the pane shows no tiles.
  $effect(() => {
    thumbs.want(thumbnails ? wantedBy(view, visible, near, thumbs.size) : []);
  });

  // The values of the extra columns to fetch are those of the files in view, then those near it; none for a column that is off.
  $effect(() => {
    extras.want(wantedExtras(view, visible, near, extraColumns));
  });

  // The active pane's list holds the keyboard focus (Tab, a click or the start of the app make a pane active),
  // and keeps it when another tab is shown (the tab button that was clicked may be gone by then). A pane that
  // becomes active with a menu open leaves the focus to the menu.
  $effect(() => {
    void tabs.active;
    if (active && untrack(() => menu) === undefined) {
      list?.focus();
    }
  });

  /** A menu closed from inside brings the focus back to the list; one closed by a click elsewhere leaves it there. */
  function closeMenu(refocus: boolean) {
    onMenu(undefined);
    if (refocus) {
      list?.focus();
    }
  }

  function onkeydown(event: KeyboardEvent) {
    if (handleKey(view, event, pageSize, undefined, perRow)) {
      event.preventDefault();
    }
  }

  /** The row a mouse event hit (the list handles the mouse for all its rows). */
  function rowOf(event: MouseEvent): number | undefined {
    const option = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-row]') : null;
    return option ? Number(option.dataset.row) : undefined;
  }

  /** A click in the rename field is for the field: it neither moves the cursor nor takes the focus away. */
  const inField = (event: MouseEvent) => event.target instanceof Element && event.target.closest('input') !== null;

  function onclick(event: MouseEvent) {
    const row = rowOf(event);
    if (row === undefined || inField(event)) {
      return;
    }

    if (event.shiftKey) {
      view.pick(row, 'range');
    } else if (event.ctrlKey || event.metaKey) {
      view.pick(row, 'toggle');
    } else {
      view.pick(row, 'cursor');
    }

    list?.focus();
  }

  function ondblclick(event: MouseEvent) {
    // Ctrl/Cmd/Shift clicks select (the clicks of a double click already did); they do not open.
    if (event.ctrlKey || event.metaKey || event.shiftKey || inField(event)) {
      return;
    }

    const row = rowOf(event);
    if (row !== undefined) {
      void view.open(row);
    }
  }

  // Clicking anywhere in a pane makes it the active one and, unless it hit a control or a menu, focuses its list.
  function activateOnClick(node: HTMLElement) {
    const onclick = (event: MouseEvent) => {
      onActivate();
      if (!(event.target instanceof Element && event.target.closest('button, input, [data-menu]'))) {
        list?.focus();
      }
    };

    node.addEventListener('click', onclick);
    return () => node.removeEventListener('click', onclick);
  }
</script>

<section
  class="pane"
  data-pane
  class:active
  aria-label={label}
  aria-current={active ? 'true' : undefined}
  aria-busy={listing.status === 'loading'}
  style:--cols-full={tracks.full}
  style:--cols-narrow={tracks.narrow}
  style:--rename-end={shows('ext', preferences.value) ? 4 : 3}
  {@attach activateOnClick}
>
  <PaneHeader
    {tabs}
    volumes={volumes.list}
    flat={flat?.shown}
    driveMenuOpen={menu === 'drives'}
    onDriveMenu={() => onMenu(menu === 'drives' ? undefined : 'drives')}
  />

  {#if menu === 'drives'}
    <DriveMenu {tabs} {volumes} onClose={closeMenu} />
  {:else if menu === 'hotlist'}
    <HotlistMenu {tabs} {hotlist} onClose={closeMenu} />
  {/if}

  {#if detailed}
    <ColumnHeader {view} />
  {/if}

  <div class="rows">
    {#if listing.status === 'error'}
      <p class="message" role="alert">{displayName(listing.error ?? '')}</p>
    {:else}
      <div
        class="list"
        role="listbox"
        tabindex="0"
        aria-label="{label} files"
        aria-multiselectable="true"
        aria-activedescendant={view.cursor >= 0 ? `${uid}-${view.cursor}` : undefined}
        bind:this={list}
        {onkeydown}
        {onclick}
        {ondblclick}
      >
        {#snippet row(index: number)}
          {@const entry = view.entryAt(index)}
          <PaneRow
            id="{uid}-{index}"
            {index}
            {entry}
            {brief}
            tile={thumbnails}
            thumb={thumbnails ? thumbs.get(targetOf(listing, entry, thumbs.size)) : undefined}
            extra={extraColumns.dimensions || extraColumns.checksum || extraColumns.git
              ? extras.cells(extraTargetOf(listing, entry), whereIs(listing, entry))
              : undefined}
            up={view.isUp(index)}
            cursor={index === view.cursor}
            selected={entry !== undefined && view.isSelected(entry.name)}
            size={entry && view.sizes.text(entry)}
            mark={entry && view.marks.get(entry.name)}
            edit={editing !== undefined && entry?.name === editing.name ? rename : undefined}
          />
        {/snippet}

        <!-- A new folder starts at the top; a reload of the same folder keeps the scroll position. -->
        {#key listing.location}
          {#if thumbnails}
            <VirtualGrid count={view.count} active={view.cursor} bind:pageSize bind:perRow bind:visible bind:near tile={row} />
          {:else if brief}
            <VirtualColumns count={view.count} active={view.cursor} bind:pageSize {row} />
          {:else}
            <VirtualRows count={view.count} active={view.cursor} bind:pageSize bind:visible bind:near {row} />
          {/if}
        {/key}
      </div>
      {#if listing.notice}
        <p class="notice" role="alert">{displayName(listing.notice)}</p>
      {:else if flat?.running}
        <p class="notice progress" role="status">Listing all files below this folder… (Ctrl+B or Esc stops)</p>
      {/if}
      {#if listing.slow}
        <div class="spinner" role="status" aria-label="Loading"></div>
      {:else if listing.status === 'ready' && listing.entries.length === 0}
        <p class="message">This folder is empty.</p>
      {/if}
    {/if}
  </div>

  <PaneFooter {view} onclosemask={() => list?.focus()} />
</section>

<style>
  .pane {
    /* The columns of the header and of every row: icon, name, then ext, size, modified, dimensions, checksum and git as far as the settings show them. */
    --pane-cols: var(--cols-full);
    --cursor-line: color-mix(in srgb, var(--accent) 35%, transparent);
    --cursor-bg: color-mix(in srgb, var(--text) 5%, transparent);
    flex: 1;
    display: flex;
    flex-direction: column;
    position: relative;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--panel);
    transition: border-color 0.12s;
  }

  .pane.active {
    --cursor-line: var(--accent);
    --cursor-bg: color-mix(in srgb, var(--accent) 16%, transparent);
    border-color: color-mix(in srgb, var(--accent) 60%, var(--line));
  }

  .rows {
    position: relative;
    flex: 1;
    min-height: 0;
  }

  .list {
    height: 100%;
    outline: none;
  }

  .message {
    margin: 0;
    padding: 28px 16px;
    color: var(--dim);
    text-align: center;
  }

  .notice {
    position: absolute;
    right: 10px;
    bottom: 8px;
    left: 10px;
    margin: 0;
    padding: 6px 10px;
    border: 1px solid color-mix(in srgb, var(--danger) 40%, var(--line));
    border-radius: 6px;
    background: var(--panel-2);
    color: var(--danger);
    font-size: 12px;
  }

  /* What goes on, not what went wrong. */
  .notice.progress {
    border-color: var(--line);
    color: var(--dim);
  }

  .spinner {
    position: absolute;
    top: 50%;
    left: 50%;
    width: 22px;
    height: 22px;
    margin: -11px 0 0 -11px;
    border: 2px solid var(--line);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(1turn);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation-duration: 2.4s;
    }
  }

  @media (max-width: 760px) {
    .pane {
      --pane-cols: var(--cols-narrow);
    }
  }
</style>
