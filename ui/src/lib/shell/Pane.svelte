<script lang="ts">
  import ColumnHeader from '$lib/panes/ColumnHeader.svelte';
  import { handleKey } from '$lib/panes/keys';
  import PaneFooter from '$lib/panes/PaneFooter.svelte';
  import PaneRow from '$lib/panes/PaneRow.svelte';
  import VirtualRows from '$lib/panes/VirtualRows.svelte';
  import type { PaneView } from '$lib/panes/view.svelte';

  let {
    label,
    view,
    active,
    onActivate,
  }: {
    /** Accessible name of the pane ("Left pane"). */
    label: string;
    view: PaneView;
    active: boolean;
    onActivate: () => void;
  } = $props();

  const listing = $derived(view.listing);
  const uid = $props.id();
  let list = $state<HTMLElement>();
  let pageSize = $state(1);

  // The active pane's list holds the keyboard focus (Tab, a click or the start of the app make a pane active).
  $effect(() => {
    if (active) {
      list?.focus();
    }
  });

  /** A root crumb (`/`, `C:\\`) already ends in a separator; no second one follows it. */
  const isRoot = (name: string) => name.endsWith('/') || name.endsWith('\\');

  /** Long paths scroll so the current folder (the end) stays in view. */
  function showEnd(node: HTMLElement, _path: string | undefined) {
    const scroll = () => (node.scrollLeft = node.scrollWidth);
    scroll();
    return { update: scroll };
  }

  function onkeydown(event: KeyboardEvent) {
    if (handleKey(view, event, pageSize)) {
      event.preventDefault();
    }
  }

  /** The row a mouse event hit (the list handles the mouse for all its rows). */
  function rowOf(event: MouseEvent): number | undefined {
    const option = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-row]') : null;
    return option ? Number(option.dataset.row) : undefined;
  }

  function onclick(event: MouseEvent) {
    const row = rowOf(event);
    if (row === undefined) {
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
    if (event.ctrlKey || event.metaKey || event.shiftKey) {
      return;
    }

    const row = rowOf(event);
    if (row !== undefined) {
      void view.open(row);
    }
  }

  // Clicking anywhere in a pane makes it the active one and, unless it hit a control, focuses its list.
  function activateOnClick(node: HTMLElement) {
    const onclick = (event: MouseEvent) => {
      onActivate();
      if (!(event.target instanceof Element && event.target.closest('button, input'))) {
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
  {@attach activateOnClick}
>
  <div class="phead">
    <button class="drive" type="button" aria-label="Drives">
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 7.5 8 3l5.5 4.5V13h-4v-3h-3v3h-4z" /></svg>
      <span class="caret">▾</span>
    </button>
    <!-- Tab roles arrive with real tabs (#9); until then the active tab is just the path. -->
    <div class="tabs">
      <div class="tab on">
        <nav class="crumbs" aria-label="Path" use:showEnd={listing.path}>
          {#each listing.crumbs as crumb, index (crumb.path)}
            {#if index > 0 && !isRoot(listing.crumbs[index - 1].name)}<i>/</i>{/if}
            <button
              type="button"
              class:last={index === listing.crumbs.length - 1}
              onclick={() => void listing.open(crumb.path)}>{crumb.name}</button
            >
          {/each}
        </nav>
      </div>
      <button class="tab add" type="button" aria-label="New tab">+</button>
    </div>
  </div>

  <ColumnHeader {view} />

  <div class="rows">
    {#if listing.status === 'error'}
      <p class="message" role="alert">{listing.error}</p>
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
        <!-- A new folder starts at the top; a reload of the same folder keeps the scroll position. -->
        {#key listing.path}
          <VirtualRows count={view.count} active={view.cursor} bind:pageSize>
            {#snippet row(index)}
              {@const entry = view.entryAt(index)}
              <PaneRow
                id="{uid}-{index}"
                {index}
                {entry}
                up={view.isUp(index)}
                cursor={index === view.cursor}
                selected={entry !== undefined && view.isSelected(entry.name)}
                size={entry && view.sizes.text(entry)}
              />
            {/snippet}
          </VirtualRows>
        {/key}
      </div>
      {#if listing.notice}
        <p class="notice" role="alert">{listing.notice}</p>
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
    /* The columns of the header and of every row: icon, name, ext, size, modified. */
    --pane-cols: 20px minmax(0, 1fr) 48px 74px 118px;
    --cursor-line: color-mix(in srgb, var(--accent) 35%, transparent);
    --cursor-bg: color-mix(in srgb, var(--text) 5%, transparent);
    flex: 1;
    display: flex;
    flex-direction: column;
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

  /* One header row per pane: one height, one font size. */
  .phead {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 40px;
    padding: 0 6px;
    border-bottom: 1px solid var(--line);
    background: var(--panel-2);
    font-size: 12.5px;
  }

  .drive {
    flex: none;
    height: 28px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 6px 0 8px;
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--panel);
    color: var(--text);
  }

  .drive:hover,
  .active .drive {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--line));
  }

  .drive svg {
    width: 15px;
    height: 15px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linejoin: round;
  }

  .caret {
    font-size: 9px;
    color: var(--dim);
  }

  .tabs {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 4px;
    overflow: hidden;
  }

  .tab {
    flex: none;
    height: 28px;
    display: flex;
    align-items: center;
    padding: 0 9px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: none;
    color: var(--dim);
  }

  .tab.on {
    flex: 1 1 auto;
    min-width: 0;
    padding: 0 2px;
    border-color: var(--line);
    background: var(--panel);
    color: var(--text);
  }

  .crumbs {
    display: flex;
    align-items: center;
    gap: 1px;
    overflow-x: auto;
    scrollbar-width: none;
    white-space: nowrap;
  }

  .crumbs button {
    flex: none;
    padding: 2px 5px;
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--dim);
  }

  .crumbs button:hover {
    background: var(--panel-2);
    color: var(--text);
  }

  .crumbs button.last {
    color: var(--text);
    font-weight: 600;
  }

  .crumbs i {
    font-style: normal;
    color: var(--line);
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
      --pane-cols: 20px minmax(0, 1fr) 74px;
    }
  }
</style>
