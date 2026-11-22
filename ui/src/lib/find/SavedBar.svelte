<script lang="ts">
  import type { FindCriteria } from '$lib/ipc/bindings/FindCriteria';
  import { displayName } from '$lib/panes/format';

  import { MAX_SEARCH_NAME, type SavedSearches } from './saved.svelte';
  import '../ops/fields.css';

  let {
    saved,
    criteria,
    onLoad,
  }: {
    saved: SavedSearches;
    /** What the form asks for now, or why it cannot be searched. */
    criteria: () => FindCriteria | string;
    /** A saved search was chosen to be loaded into the form. */
    onLoad: (criteria: FindCriteria) => void;
  } = $props();

  let choice = $state('');
  let naming = $state(false);
  let name = $state('');
  let message = $state<string | undefined>();

  const chosen = $derived(saved.find(choice));

  function load() {
    if (chosen !== undefined) {
      message = undefined;
      onLoad(chosen.criteria);
    }
  }

  function remove() {
    saved.remove(choice);
    choice = '';
    message = undefined;
  }

  function beginNaming() {
    name = choice;
    naming = true;
    message = undefined;
  }

  function save() {
    const asked = criteria();
    const refusal = typeof asked === 'string' ? asked : saved.save(name, asked);
    if (refusal !== undefined) {
      message = refusal;
      return;
    }

    choice = name.trim();
    naming = false;
  }

  /** Enter saves here, not searches; Escape gives up the name, not the dialog. */
  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'Enter') {
      event.preventDefault();
      event.stopPropagation();
      save();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      naming = false;
    }
  }

  // A listener of the field itself: the dialog's Enter, which searches, is on an ancestor and would see the key first.
  function watch(input: HTMLInputElement) {
    input.focus();
    input.select();
    input.addEventListener('keydown', onkeydown);
    return () => input.removeEventListener('keydown', onkeydown);
  }
</script>

<div class="saved" role="group" aria-label="Saved searches">
  <select class="field" aria-label="Saved search" value={choice} onchange={(event) => (choice = event.currentTarget.value)}>
    <option value="">{saved.list.length === 0 ? 'No saved searches' : 'Saved searches…'}</option>
    {#each saved.list as { name: kept } (kept)}
      <option value={kept}>{displayName(kept)}</option>
    {/each}
  </select>
  <button class="btn" type="button" disabled={chosen === undefined} onclick={load}>Load</button>
  <button class="btn" type="button" disabled={chosen === undefined} onclick={remove}>Delete</button>
  {#if naming}
    <input
      class="field name"
      type="text"
      spellcheck="false"
      autocomplete="off"
      maxlength={MAX_SEARCH_NAME}
      aria-label="Name of the search"
      bind:value={name}
      {@attach watch}
    />
    <button class="btn btn-primary" type="button" onclick={save}>Save</button>
    <button class="btn" type="button" onclick={() => (naming = false)}>Cancel</button>
  {:else}
    <button class="btn" type="button" onclick={beginNaming}>Save as…</button>
  {/if}
</div>
{#if message}
  <p class="failure" role="alert">{displayName(message)}</p>
{/if}

<style>
  .saved {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  select {
    min-width: 170px;
  }

  .name {
    flex: 1;
    min-width: 120px;
  }

  .failure {
    margin: 0;
    color: var(--danger);
  }
</style>
