<script lang="ts">
  import { formatSize } from '$lib/panes/format';
  import '$lib/shell/overlayBar.css';

  import { encodingChoices } from './encodings';
  import type { ViewSearch } from './search.svelte';
  import type { ViewMode, ViewSession } from './session.svelte';

  let {
    session,
    search,
    onClose,
  }: {
    session: ViewSession;
    search: ViewSearch;
    onClose: () => void;
  } = $props();

  const LABELS: Record<ViewMode, string> = { auto: 'Preview', text: 'Text', hex: 'Hex' };

  const searchable = $derived(session.shown === 'text' || session.shown === 'hex');

  /** What the search says: how far a long one has got, that nothing was found, why it failed, how to write hex. */
  const message = $derived.by(() => {
    if (search.searching) {
      return search.progress > 0 ? `Searching… ${formatSize(search.progress)}` : 'Searching…';
    }

    if (search.invalid) {
      return 'Hex bytes, like 4d 5a';
    }

    if (search.status === 'none') {
      return 'Not found';
    }

    return search.status === 'failed' ? search.failure : undefined;
  });

  function onSearchKey(event: KeyboardEvent) {
    if (event.key === 'Enter') {
      event.preventDefault();
      search.run(event.shiftKey);
    } else if (event.key === 'Escape' && search.searching) {
      // Stops the search; a second Escape closes the viewer.
      event.preventDefault();
      event.stopPropagation();
      search.cancel();
    }
  }
</script>

<div class="overlay-bar" role="toolbar" aria-label="Viewer">
  <span class="overlay-bar-name" aria-hidden="true">{session.name}</span>
  {#if session.status === 'ready'}
    <span class="size">{formatSize(session.size)}</span>

    <div class="group" role="group" aria-label="View as">
      {#each session.modes as mode (mode)}
        <button type="button" aria-pressed={session.marked === mode} onclick={() => (session.mode = mode)}>
          {LABELS[mode]}
        </button>
      {/each}
    </div>

    {#if session.decoded}
      <select aria-label="Encoding" value={session.encoding} onchange={(event) => (session.encoding = event.currentTarget.value)}>
        {#each encodingChoices(session.opened?.encoding ?? '') as encoding (encoding)}
          <option value={encoding}>{encoding}</option>
        {/each}
      </select>
    {/if}

    {#if session.shown === 'text'}
      <button type="button" aria-pressed={session.wrap} onclick={() => (session.wrap = !session.wrap)}>Wrap</button>
    {/if}

    {#if searchable}
      <div class="search" role="search">
        <input
          type="text"
          data-search
          aria-label="Search"
          aria-invalid={search.invalid}
          placeholder={search.hex ? 'Hex bytes…' : 'Search…'}
          spellcheck="false"
          bind:value={search.text}
          oninput={() => search.edited()}
          onkeydown={onSearchKey}
        />
        <button type="button" aria-label="Previous match" onclick={() => search.run(true)}>↑</button>
        <button type="button" aria-label="Next match" onclick={() => search.run(false)}>↓</button>
        <button
          type="button"
          aria-pressed={search.caseSensitive}
          aria-label="Match case"
          disabled={search.hex}
          title="Match case"
          onclick={() => {
            search.caseSensitive = !search.caseSensitive;
            search.edited();
          }}>Aa</button
        >
        <button
          type="button"
          aria-pressed={search.hex}
          aria-label="Search for hex bytes"
          title="Search for hex bytes"
          onclick={() => {
            search.hex = !search.hex;
            search.edited();
          }}>Hex</button
        >
        {#if search.searching}
          <button type="button" onclick={() => search.cancel()}>Cancel</button>
        {/if}
        <span class="message" role="status">{message ?? ''}</span>
      </div>
    {/if}
  {/if}

  <button class="close" type="button" aria-label="Close" onclick={onClose}>Close <kbd>Esc</kbd></button>
</div>

<style>
  .size,
  .message {
    color: var(--dim);
    font-size: 12px;
  }

  .group,
  .search {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .search {
    flex: 1;
    min-width: 280px;
  }

  button,
  select,
  input {
    height: 26px;
    padding: 0 9px;
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--panel-2);
    color: inherit;
    font: inherit;
  }

  button:hover:not(:disabled),
  button[aria-pressed='true'] {
    border-color: color-mix(in srgb, var(--accent) 60%, var(--line));
    background: color-mix(in srgb, var(--accent) 16%, var(--panel-2));
  }

  button:disabled {
    opacity: 0.5;
  }

  input {
    flex: 1;
    min-width: 120px;
    max-width: 320px;
    outline: none;
  }

  input:focus-visible {
    border-color: var(--accent);
  }

  input[aria-invalid='true'] {
    border-color: var(--danger);
  }

  .close {
    margin-left: auto;
  }
</style>
