<script lang="ts">
  import type { Entry } from '$lib/ipc/bindings/Entry';
  import { formatDate, formatSize, formatSummary, iconClass, splitName } from '$lib/panes/format';
  import type { Listing } from '$lib/panes/listing.svelte';
  import VirtualRows from '$lib/panes/VirtualRows.svelte';

  let {
    label,
    listing,
    active,
    onActivate,
  }: {
    /** Accessible name of the pane ("Left pane"). */
    label: string;
    listing: Listing;
    active: boolean;
    onActivate: () => void;
  } = $props();

  /** The `..` row leads to the parent folder; it comes first and is not an entry. */
  const up = $derived(listing.parent !== null && listing.status !== 'error');
  const count = $derived(listing.entries.length + (up ? 1 : 0));

  function entryAt(index: number): Entry | undefined {
    return listing.entries[up ? index - 1 : index];
  }

  /** A root crumb (`/`, `C:\\`) already ends in a separator; no second one follows it. */
  const isRoot = (name: string) => name.endsWith('/') || name.endsWith('\\');

  /** Long paths scroll so the current folder (the end) stays in view. */
  function showEnd(node: HTMLElement, _path: string | undefined) {
    const scroll = () => (node.scrollLeft = node.scrollWidth);
    scroll();
    return { update: scroll };
  }

  function openRow(index: number) {
    if (up && index === 0) {
      void listing.open(listing.parent);
      return;
    }

    const entry = entryAt(index);
    if (entry?.kind === 'dir') {
      void listing.enter(entry.name);
    }
  }
</script>

<!-- Clicking anywhere in a pane makes it the active one; Tab does the same from the keyboard (App). -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<section
  class="pane"
  data-pane
  class:active
  aria-label={label}
  aria-current={active ? 'true' : undefined}
  aria-busy={listing.status === 'loading'}
  onclick={onActivate}
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

  <div class="head">
    <span></span><span>Name</span><span>Ext</span><span class="r">Size</span><span>Modified</span>
  </div>

  <div class="rows">
    {#if listing.status === 'error'}
      <p class="message" role="alert">{listing.error}</p>
    {:else}
      <!-- A new folder starts at the top; a reload of the same folder keeps the scroll position. -->
      {#key listing.path}
        <VirtualRows {count}>
          {#snippet row(index)}
            {@const entry = entryAt(index)}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div class="row" class:d={entry?.kind === 'dir'} class:h={entry?.hidden} ondblclick={() => openRow(index)}>
              {#if up && index === 0}
                <span class="ic up">↰</span><span class="nm">..</span>
              {:else if entry}
                {@const { base, ext } = splitName(entry)}
                <span class="ic {iconClass(entry)}"></span>
                <span class="nm" title={entry.linkTarget ? `${entry.name} → ${entry.linkTarget}` : undefined}>{base}</span>
                <span class="ex">{ext}</span>
                <span class="sz">{#if entry.kind === 'dir'}<span class="dirtag">DIR</span>{:else}{formatSize(entry.size)}{/if}</span>
                <span class="dt">{formatDate(entry.modified)}</span>
              {/if}
            </div>
          {/snippet}
        </VirtualRows>
      {/key}
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

  <div class="foot">
    {#if listing.summary}
      {formatSummary(listing.summary.dirs, listing.summary.files, listing.summary.bytes)}
    {/if}
  </div>
</section>

<style>
  .pane {
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

  .head,
  .row {
    display: grid;
    grid-template-columns: 20px minmax(0, 1fr) 48px 74px 118px;
    align-items: center;
    gap: 8px;
    padding: 0 10px;
  }

  .head {
    height: 25px;
    border-bottom: 1px solid var(--line);
    font-size: 11px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--dim);
    user-select: none;
  }

  .head .r {
    text-align: right;
  }

  .rows {
    position: relative;
    flex: 1;
    min-height: 0;
  }

  .row {
    height: 24px;
    border-left: 2px solid transparent;
    cursor: default;
    user-select: none;
  }

  .row:hover {
    background: color-mix(in srgb, var(--text) 4%, transparent);
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

  .foot {
    height: 28px;
    display: flex;
    align-items: center;
    padding: 0 10px;
    border-top: 1px solid var(--line);
    font-size: 12px;
    color: var(--dim);
    white-space: nowrap;
    overflow: hidden;
  }

  @media (max-width: 760px) {
    .head,
    .row {
      grid-template-columns: 20px minmax(0, 1fr) 74px;
    }

    .head span:nth-child(3),
    .head span:nth-child(5),
    .ex,
    .dt {
      display: none;
    }
  }
</style>
