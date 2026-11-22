<script lang="ts">
  import type { FindHit } from '$lib/ipc/bindings/FindHit';
  import { displayName, formatDate, formatSize } from '$lib/panes/format';
  import VirtualRows from '$lib/panes/VirtualRows.svelte';

  import type { Finder } from './finder.svelte';

  let {
    finder,
    selected = $bindable(-1),
    onOpen,
  }: {
    finder: Finder;
    /** The row with the cursor (always a result, never the head of a group); `-1` when there is none. */
    selected?: number;
    /** Enter or a double click on a result. */
    onOpen: (hit: FindHit) => void;
  } = $props();

  const uid = $props.id();
  let pageSize = $state(1);

  /** The nearest row at or after `from` (in `direction`) that is a result. */
  function hitFrom(from: number, direction: 1 | -1): number | undefined {
    const rows = finder.rows;
    for (let at = from; at >= 0 && at < rows.length; at += direction) {
      if (rows[at].kind === 'hit') {
        return at;
      }
    }

    return undefined;
  }

  function hitAt(index: number): FindHit | undefined {
    const row = finder.rows[index];
    return row?.kind === 'hit' ? row.hit : undefined;
  }

  // The cursor stays on a result: it goes to the first one when there is none (the results of a new search).
  $effect(() => {
    if (hitAt(selected) === undefined) {
      selected = hitFrom(0, 1) ?? -1;
    }
  });

  function move(key: string): number | undefined {
    const last = finder.rows.length - 1;
    switch (key) {
      case 'ArrowDown':
        return hitFrom(selected + 1, 1);
      case 'ArrowUp':
        return hitFrom(selected - 1, -1);
      case 'PageDown':
        return hitFrom(Math.min(selected + pageSize, last), -1);
      case 'PageUp':
        return hitFrom(Math.max(selected - pageSize, 0), 1);
      case 'Home':
        return hitFrom(0, 1);
      case 'End':
        return hitFrom(last, -1);
      default:
        return undefined;
    }
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.altKey || event.ctrlKey || event.metaKey || event.isComposing) {
      return;
    }

    if (event.key === 'Enter') {
      const hit = hitAt(selected);
      if (hit !== undefined) {
        event.preventDefault();
        onOpen(hit);
      }
      return;
    }

    if (['ArrowDown', 'ArrowUp', 'PageDown', 'PageUp', 'Home', 'End'].includes(event.key)) {
      event.preventDefault();
      selected = move(event.key) ?? selected;
    }
  }

  function rowOf(event: MouseEvent): number | undefined {
    const row = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-row]') : null;
    return row === null ? undefined : Number(row.dataset.row);
  }

  function onclick(event: MouseEvent) {
    const row = rowOf(event);
    if (row !== undefined) {
      selected = row;
    }
  }

  function ondblclick(event: MouseEvent) {
    const hit = hitAt(rowOf(event) ?? -1);
    if (hit !== undefined) {
      onOpen(hit);
    }
  }

  const EMPTY = { idle: 'The results appear here.', running: 'Looking…', done: 'Nothing found.', failed: '' } as const;
</script>

<div class="results">
  {#if finder.rows.length === 0}
    <p class="message">{EMPTY[finder.status]}</p>
  {:else}
    <div
      class="list"
      role="listbox"
      tabindex="0"
      aria-label="Results"
      aria-activedescendant={selected >= 0 ? `${uid}-${selected}` : undefined}
      {onkeydown}
      {onclick}
      {ondblclick}
    >
      <VirtualRows count={finder.rows.length} active={selected} bind:pageSize>
        {#snippet row(index)}
          {@const item = finder.rows[index]}
          {#if item?.kind === 'group'}
            <div class="group" role="presentation">{item.count} files of {formatSize(item.size)} each</div>
          {:else if item?.kind === 'hit'}
            {@const hit = item.hit}
            <div id="{uid}-{index}" class="row" class:cursor={index === selected} role="option" aria-selected={index === selected} data-row={index}>
              <span class="path" title={displayName(hit.inner === null ? hit.path : `${hit.path} › ${hit.inner}`)}
                >{displayName(hit.path)}{#if hit.inner !== null}<span class="inner"> › {displayName(hit.inner)}</span>{/if}</span
              >
              <span class="sz">{hit.kind === 'file' ? formatSize(hit.size) : ''}</span>
              <span class="dt">{formatDate(hit.modified)}</span>
              <span class="match">
                {#if hit.matches !== null}
                  <span class="count">{hit.matches}</span>
                  {#if hit.line !== null}
                    <span class="line">{hit.lineNumber ?? ''}: {hit.line}</span>
                  {/if}
                {/if}
              </span>
            </div>
          {/if}
        {/snippet}
      </VirtualRows>
    </div>
  {/if}
</div>

<style>
  .results {
    height: 220px;
    border: 1px solid var(--line);
    border-radius: 7px;
    background: var(--bg);
    overflow: hidden;
  }

  .list {
    height: 100%;
    outline: none;
  }

  .list:focus-visible {
    box-shadow: inset 0 0 0 1px var(--accent);
  }

  .message {
    margin: 0;
    padding: 24px 16px;
    color: var(--dim);
    text-align: center;
  }

  .row,
  .group {
    height: 24px;
    padding: 0 10px;
    border-left: 2px solid transparent;
    user-select: none;
  }

  .row {
    display: grid;
    grid-template-columns: minmax(0, 1.4fr) 64px 112px minmax(0, 1fr);
    align-items: center;
    gap: 8px;
    cursor: default;
  }

  .row:hover {
    background: color-mix(in srgb, var(--text) 4%, transparent);
  }

  .row.cursor {
    border-left-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }

  .group {
    display: flex;
    align-items: center;
    background: var(--panel-2);
    color: var(--dim);
    font-size: 12px;
  }

  .path,
  .dt,
  .match {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* A path may hold any characters, right-to-left ones too: it is set apart from the text around it. */
  .path {
    unicode-bidi: isolate;
  }

  .inner {
    color: var(--dim);
  }

  .sz {
    font: 11.5px var(--font-mono);
    color: var(--dim);
    text-align: right;
    white-space: nowrap;
  }

  .dt,
  .match {
    font-size: 12px;
    color: var(--dim);
  }

  .count {
    margin-right: 6px;
    color: var(--accent-2);
  }

  .line {
    unicode-bidi: isolate;
    font-family: var(--font-mono);
  }
</style>
