<script lang="ts" generics="T extends { name: string }, V">
  import { displayName } from '$lib/panes/format';

  import { MAX_SAVED_NAME, type SavedNamed } from './named.svelte';
  import '../ops/fields.css';

  let {
    saved,
    current,
    onLoad,
  }: {
    saved: SavedNamed<T, V>;
    /** What the form holds now, or why it cannot be kept. */
    current: () => V | string;
    /** A saved item was chosen to be loaded into the form. */
    onLoad: (value: V) => void;
  } = $props();

  const one = $derived(saved.words.one);
  const many = $derived(saved.words.many);

  let choice = $state('');
  let naming = $state(false);
  let name = $state('');
  let message = $state<string | undefined>();

  const chosen = $derived(saved.find(choice));

  function load() {
    if (chosen !== undefined) {
      message = undefined;
      onLoad(saved.valueFor(chosen.name) as V);
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
    const asked = current();
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

<div class="saved" role="group" aria-label="Saved {many}">
  <select class="field" aria-label="Saved {one}" value={choice} onchange={(event) => (choice = event.currentTarget.value)}>
    <option value="">{saved.list.length === 0 ? `No saved ${many}` : `Saved ${many}…`}</option>
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
      maxlength={MAX_SAVED_NAME}
      aria-label="Name of the {one}"
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
