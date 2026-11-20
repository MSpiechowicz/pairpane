<script lang="ts">
  import type { Hotlist } from '$lib/panes/hotlist.svelte';
  import type { PaneTabs } from '$lib/panes/tabs.svelte';

  import Dropdown from './Dropdown.svelte';

  let {
    tabs,
    hotlist,
    onClose,
  }: {
    tabs: PaneTabs;
    hotlist: Hotlist;
    onClose: (refocus: boolean) => void;
  } = $props();

  const listing = $derived(tabs.view.listing);
  const here = $derived(listing.path);

  let field = $state<HTMLInputElement>();
  let query = $state('');
  let active = $state(0);
  const matches = $derived(hotlist.filter(query));

  function choose(index: number) {
    void listing.open(matches[index]);
    onClose(true);
  }

  /** Adds the folder the pane shows, or takes it off when it is in the list. The filter gets the focus (and the keys) back. */
  function toggleHere() {
    if (here === undefined) {
      return;
    }

    if (hotlist.has(here)) {
      hotlist.remove(here);
    } else {
      hotlist.add(here);
    }

    field?.focus();
  }
</script>

<Dropdown
  label="Favourite folders"
  count={matches.length}
  bind:active
  empty={query === '' ? 'No favourite folders yet.' : 'No favourite folder matches.'}
  onChoose={choose}
  onRemove={(index) => hotlist.remove(matches[index])}
  removeLabel="Remove from favourites"
  {onClose}
>
  {#snippet header(listId, activeId)}
    <input
      type="text"
      role="combobox"
      aria-label="Filter favourite folders"
      aria-expanded="true"
      aria-controls={listId}
      aria-activedescendant={activeId}
      autocomplete="off"
      spellcheck="false"
      placeholder="Filter favourites…"
      bind:this={field}
      bind:value={query}
      oninput={() => (active = 0)}
    />
  {/snippet}

  {#snippet row(index)}
    <span class="fit">{matches[index]}</span>
  {/snippet}

  {#snippet footer()}
    <button class="here" type="button" disabled={here === undefined} onclick={toggleHere}>
      {here !== undefined && hotlist.has(here) ? 'Remove current folder' : 'Add current folder'}
    </button>
  {/snippet}
</Dropdown>

<style>
  input {
    width: 100%;
    height: 28px;
    margin-bottom: 4px;
    padding: 0 8px;
    border: 1px solid var(--line);
    border-radius: 6px;
    outline: none;
    background: var(--bg);
    color: var(--text);
  }

  input:focus {
    border-color: var(--accent);
  }

  .here {
    width: 100%;
    height: 28px;
    margin-top: 4px;
    padding: 0 8px;
    border: 0;
    border-radius: 6px;
    background: none;
    color: var(--dim);
    text-align: left;
    transition:
      color 0.12s,
      background-color 0.12s;
  }

  .here:hover:not(:disabled) {
    background: var(--panel-2);
    color: var(--text);
  }
</style>
