<script lang="ts">
  import { displayName } from '$lib/panes/format';
  import Modal from '$lib/shell/Modal.svelte';

  import type { Palette, PaletteItem } from './model.svelte';

  let { palette, onTake }: { palette: Palette; onTake: (item: PaletteItem) => void } = $props();

  const items = $derived(palette.items);
  const listId = $props.id();

  /** What a row says it is, besides its name. */
  const KIND_TEXT: Record<PaletteItem['kind'], string> = { command: '', folder: 'Folder', file: 'Here' };

  function take(index?: number) {
    const item = palette.take(index);
    if (item !== undefined) {
      onTake(item);
    }
  }

  function onKeyDown(event: KeyboardEvent) {
    // A key of an input method that composes text (keyCode 229 in WebKit) is the method's, not the palette's.
    if (event.isComposing || event.keyCode === 229) {
      return;
    }

    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      palette.move(event.key === 'ArrowDown' ? 1 : -1);
    } else if (event.key === 'Enter') {
      event.preventDefault();
      take();
    }
  }

  /** Keeps the chosen row in view as the choice moves. */
  function follow(node: HTMLElement, chosen: boolean) {
    const show = (now: boolean) => {
      if (now) {
        node.scrollIntoView?.({ block: 'nearest' });
      }
    };
    show(chosen);

    return { update: show };
  }
</script>

<Modal title="Command palette" variant="wide" onClose={() => palette.close()}>
  <input
    class="field query"
    type="text"
    role="combobox"
    aria-expanded="true"
    aria-autocomplete="list"
    aria-controls={listId}
    aria-activedescendant={items.length > 0 ? `${listId}-${palette.cursor}` : undefined}
    aria-label="Command, folder or name"
    placeholder="Type a command, a folder or a name…"
    spellcheck="false"
    autocomplete="off"
    data-autofocus
    value={palette.query}
    oninput={(event) => palette.type(event.currentTarget.value)}
    onkeydown={onKeyDown}
  />
  <ul class="items" id={listId} role="listbox" aria-label="Matches">
    <!-- svelte-ignore a11y_click_events_have_key_events (the field's arrows and Enter choose a row: the list is its combobox's) -->
    {#each items as item, index (`${item.kind}:${item.kind === 'command' ? item.id : item.kind === 'folder' ? item.path : item.name}`)}
      <li
        id="{listId}-{index}"
        class="item"
        class:chosen={index === palette.cursor}
        role="option"
        aria-selected={index === palette.cursor}
        use:follow={index === palette.cursor}
        onmousedown={(event) => event.preventDefault()}
        onclick={() => take(index)}
      >
        <span class="label">{displayName(item.label)}</span>
        {#if item.kind === 'command' && item.keys}
          <kbd>{displayName(item.keys)}</kbd>
        {:else if item.kind !== 'command'}
          <span class="kind">{KIND_TEXT[item.kind]}</span>
        {/if}
      </li>
    {:else}
      <li class="none" role="presentation">Nothing matches.</li>
    {/each}
  </ul>
</Modal>

<style>
  .query {
    width: 100%;
    box-sizing: border-box;
  }

  .items {
    max-height: min(420px, 60vh);
    margin: 8px 0 0;
    padding: 0;
    overflow: auto;
    list-style: none;
  }

  .item {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 5px 8px;
    border-radius: 6px;
    cursor: default;
  }

  .item.chosen {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }

  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .kind,
  .none,
  kbd {
    color: var(--dim);
    font-size: 11.5px;
  }

  .none {
    padding: 5px 8px;
  }
</style>
