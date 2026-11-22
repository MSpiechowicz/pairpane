<script lang="ts">
  import type { Crumb } from '$lib/ipc/bindings/Crumb';
  import type { Volume } from '$lib/ipc/bindings/Volume';
  import { crumbKey } from '$lib/panes/archive';
  import { anchored, tabLabel } from '$lib/panes/crumbs';
  import type { PaneTabs } from '$lib/panes/tabs.svelte';

  let {
    tabs,
    volumes,
    driveMenuOpen,
    onDriveMenu,
  }: {
    tabs: PaneTabs;
    volumes: readonly Volume[];
    driveMenuOpen: boolean;
    /** The drive icon was clicked. */
    onDriveMenu: () => void;
  } = $props();

  const listing = $derived(tabs.view.listing);
  const crumbs = $derived(anchored(listing.crumbs, volumes));

  /** Long paths scroll so the current folder (the end) stays in view. */
  function showEnd(node: HTMLElement, _path: string | undefined) {
    const scroll = () => (node.scrollLeft = node.scrollWidth);
    scroll();
    return { update: scroll };
  }

  /** A step of the path bar: a real folder, or a folder inside an archive (the crumb then has `inside`). */
  function goTo(crumb: Crumb) {
    if (crumb.inside === undefined) {
      void listing.open(crumb.path);
    } else {
      void listing.openArchive({ archive: crumb.path, dir: crumb.inside });
    }
  }

  /** The middle button closes a tab, as in a browser. */
  function closeOnMiddle(event: MouseEvent, index: number) {
    if (event.button === 1) {
      event.preventDefault();
      tabs.close(index);
    }
  }
</script>

<!-- One row, one height, one font size: the drive icon, then the tabs. The active tab is the path. -->
<div class="phead">
  <button
    class="drive"
    type="button"
    aria-label="Drives"
    aria-haspopup="listbox"
    aria-expanded={driveMenuOpen}
    data-menu-opener
    onclick={onDriveMenu}
  >
    <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 7.5 8 3l5.5 4.5V13h-4v-3h-3v3h-4z" /></svg>
    <span class="caret">▾</span>
  </button>

  <!-- Not role=tab: a tab's children are presentational, and the path's buttons inside the active one are not. -->
  <div class="tabs" role="group" aria-label="Tabs">
    {#each tabs.tabs as tab, index (tab.id)}
      {#if index === tabs.active}
        <div class="tab on">
          <nav class="crumbs" aria-label="Path" use:showEnd={listing.location}>
            {#each crumbs as crumb, position (crumbKey(crumb))}
              {#if position > 0}<i>/</i>{/if}
              <button
                type="button"
                class:last={position === crumbs.length - 1 && listing.results === undefined}
                onclick={() => goTo(crumb)}>{crumb.name}</button
              >
            {/each}
            <!-- The crumbs are the folder searched; the results are what is listed in it. -->
            {#if listing.results !== undefined}
              <i>/</i><span class="found">Search results</span>
            {/if}
          </nav>
        </div>
      {:else}
        {@const label = tabLabel(tab.path, volumes)}
        <div class="tab">
          <button class="name" type="button" onclick={() => tabs.select(index)} onauxclick={(e) => closeOnMiddle(e, index)}
            >{label}</button
          >
          <button class="close" type="button" aria-label="Close {label}" onclick={() => tabs.close(index)}>×</button>
        </div>
      {/if}
    {/each}
    <button class="add" type="button" aria-label="New tab" onclick={() => tabs.add()}>+</button>
  </div>
</div>

<style>
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

  /* No boxes and no separators: spacing and colour do the work, and nothing moves, it only fades. */
  button {
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--dim);
    transition:
      color 0.12s,
      background-color 0.12s,
      opacity 0.12s;
  }

  button:hover {
    background: var(--panel);
    color: var(--text);
  }

  .drive {
    flex: none;
    height: 28px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 6px 0 8px;
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
  }

  .tabs {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 2px;
    overflow: hidden;
  }

  .tab {
    flex: 0 1 auto;
    min-width: 24px;
    height: 28px;
    display: flex;
    align-items: center;
  }

  .tab.on {
    flex: 1 1 160px;
    min-width: 80px;
  }

  .name {
    min-width: 0;
    max-width: 140px;
    height: 28px;
    padding: 0 4px 0 9px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Visible on hover; a keyboard user reaches it with the focus ring. */
  .close {
    flex: none;
    width: 18px;
    height: 18px;
    margin-right: 3px;
    padding: 0;
    opacity: 0;
    font-size: 14px;
    line-height: 1;
  }

  .tab:hover .close,
  .close:focus-visible {
    opacity: 1;
  }

  .add {
    flex: none;
    width: 28px;
    height: 28px;
    padding: 0;
    font-size: 16px;
    line-height: 1;
  }

  .crumbs {
    display: flex;
    align-items: center;
    gap: 1px;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
    white-space: nowrap;
  }

  .crumbs button {
    flex: none;
    padding: 2px 5px;
  }

  .crumbs button.last,
  .crumbs .found {
    color: var(--text);
    font-weight: 600;
  }

  .crumbs .found {
    flex: none;
    padding: 2px 5px;
  }

  .crumbs i {
    font-style: normal;
    color: var(--line);
  }
</style>
