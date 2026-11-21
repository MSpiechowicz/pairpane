<script lang="ts">
  import { formatFree, formatSelection, formatSummary } from './format';
  import type { PaneView } from './view.svelte';

  let { view, onclosemask }: { view: PaneView; onclosemask: () => void } = $props();

  const listing = $derived(view.listing);
  const selection = $derived(view.selection);
  const free = $derived(listing.summary?.free ?? null);

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'Enter') {
      view.applyMask();
    } else if (event.key === 'Escape') {
      view.cancelMask();
    } else {
      return;
    }

    event.preventDefault();
    onclosemask();
  }

  function focusAll(input: HTMLInputElement) {
    input.focus();
    input.select();
  }
</script>

<div class="foot">
  {#if view.maskMode}
    {@const label = view.maskMode === 'select' ? 'Select' : 'Deselect'}
    <label class="mask">
      {label}:
      <input
        class="pane-field"
        type="text"
        spellcheck="false"
        autocomplete="off"
        bind:value={view.maskText}
        {onkeydown}
        onblur={() => view.cancelMask()}
        {@attach focusAll}
      />
    </label>
    <span class="hint"><kbd>⏎</kbd> <kbd>Esc</kbd></span>
  {:else}
    {#if view.filter !== ''}
      <span class="badge">Filter: {view.filter} · {view.matches.toLocaleString('en-US')}</span>
    {/if}
    {#if selection}
      <span>{formatSelection(selection.dirs, selection.files, selection.bytes)}</span>
    {:else if listing.summary}
      <span>{formatSummary(listing.summary.dirs, listing.summary.files, listing.summary.bytes)}</span>
    {/if}
    {#if free !== null}
      <span class="free">{formatFree(free)}</span>
    {/if}
  {/if}
</div>

<style>
  .foot {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 28px;
    padding: 0 10px;
    border-top: 1px solid var(--line);
    font-size: 12px;
    color: var(--dim);
    white-space: nowrap;
    overflow: hidden;
  }

  .badge {
    flex: none;
    padding: 1px 7px;
    border: 1px solid color-mix(in srgb, var(--accent) 45%, var(--line));
    border-radius: 4px;
    color: var(--accent);
  }

  .free {
    margin-left: auto;
  }

  .mask {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 6px;
    min-width: 0;
    color: var(--text);
  }

  .mask input {
    flex: 1;
    min-width: 0;
  }
</style>
