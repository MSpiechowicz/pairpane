<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    label,
    count,
    active = $bindable(0),
    selected,
    empty = '',
    onChoose,
    onRemove,
    removeLabel = 'Remove',
    onClose,
    header,
    row,
    footer,
  }: {
    /** Accessible name of the list. */
    label: string;
    /** Rows in the list. */
    count: number;
    /** The highlighted row: the arrow keys and the pointer move it, Enter and a click choose it. */
    active?: number;
    /** Whether a row is the current choice; when given, such a row is marked. */
    selected?: (index: number) => boolean;
    /** Said instead of the rows when there are none. */
    empty?: string;
    onChoose: (index: number) => void;
    /** When given, a row has a remove button (shown on hover), and Shift+Delete removes the highlighted row. */
    onRemove?: (index: number) => void;
    removeLabel?: string;
    /** `refocus`: it closed from inside (Escape, a choice), so the focus goes back to where it came from. */
    onClose: (refocus: boolean) => void;
    /** Above the list, the text field that filters it. It points the field at the list and the highlighted row. */
    header?: Snippet<[listId: string, activeId: string | undefined]>;
    /** The content of one row. */
    row: Snippet<[index: number]>;
    /** Below the list. */
    footer?: Snippet;
  } = $props();

  const uid = $props.id();
  const listId = `${uid}-list`;
  const optionId = (index: number) => `${uid}-${index}`;

  let root = $state<HTMLElement>();

  // The row that is highlighted stays in range when the list shrinks (a filter, a volume that was ejected).
  const cursor = $derived(Math.max(0, Math.min(active, count - 1)));
  const activeId = $derived(count > 0 ? optionId(cursor) : undefined);

  /**
   * The keys, the pointer and the focus of the whole menu. Its text field and its list share the keys, so they are
   * handled here, on the menu, and the focus starts in the field, or in the list when there is none.
   */
  function wire(node: HTMLElement) {
    node.addEventListener('keydown', onkeydown);
    node.addEventListener('click', onclick);
    node.addEventListener('pointermove', onpointermove);
    node.querySelector<HTMLElement>('input, [role="listbox"]')?.focus();

    return () => {
      node.removeEventListener('keydown', onkeydown);
      node.removeEventListener('click', onclick);
      node.removeEventListener('pointermove', onpointermove);
    };
  }

  $effect(() => {
    if (activeId !== undefined) {
      document.getElementById(activeId)?.scrollIntoView?.({ block: 'nearest' });
    }
  });

  function onkeydown(event: KeyboardEvent) {
    if (event.isComposing) {
      return;
    }

    if (event.key === 'Escape') {
      event.preventDefault();
      onClose(true);
      return;
    }

    // A button (the footer's) is operated by its own keys.
    const target = event.target;
    if (target instanceof Element && target.closest('button')) {
      return;
    }

    const typing = target instanceof Element && target.closest('input') !== null;
    const step = { ArrowDown: 1, ArrowUp: -1 }[event.key];

    if (step !== undefined) {
      active = Math.max(0, Math.min(cursor + step, count - 1));
    } else if (!typing && event.key === 'Home') {
      active = 0;
    } else if (!typing && event.key === 'End') {
      active = Math.max(0, count - 1);
    } else if (event.key === 'Enter' && count > 0) {
      onChoose(cursor);
    } else if (event.key === 'Delete' && event.shiftKey && count > 0 && onRemove) {
      onRemove(cursor);
    } else {
      return;
    }

    event.preventDefault();
  }

  /** The row a pointer event hit. */
  function rowOf(event: Event): number | undefined {
    const option = event.target instanceof Element ? event.target.closest<HTMLElement>('[role="option"]') : null;
    return option ? Number(option.dataset.index) : undefined;
  }

  function onpointermove(event: PointerEvent) {
    active = rowOf(event) ?? active;
  }

  function onclick(event: MouseEvent) {
    const index = rowOf(event);
    if (index !== undefined) {
      onChoose(index);
    }
  }

  // A click on whatever opened the menu is that control's own business (it toggles the menu).
  function closeOnOutside(event: MouseEvent) {
    const target = event.target;
    if (root && target instanceof Element && !root.contains(target) && !target.closest('[data-menu-opener]')) {
      onClose(false);
    }
  }
</script>

<svelte:window onclick={closeOnOutside} />

<div class="popup menu" data-menu bind:this={root} {@attach wire}>
  {@render header?.(listId, activeId)}

  <div
    class="list"
    class:marks={selected !== undefined}
    id={listId}
    role="listbox"
    tabindex="-1"
    aria-label={label}
    aria-activedescendant={activeId}
  >
    {#each { length: count } as _, index (index)}
      <div class="item" role="presentation">
        <div
          class="option"
          class:on={index === cursor}
          id={optionId(index)}
          role="option"
          data-index={index}
          aria-selected={selected?.(index) ?? false}
        >
          {@render row(index)}
        </div>
        {#if onRemove}
          <button class="remove" type="button" tabindex="-1" aria-label={removeLabel} onclick={() => onRemove(index)}>×</button>
        {/if}
      </div>
    {:else}
      <p class="none">{empty}</p>
    {/each}
  </div>

  {@render footer?.()}
</div>

<style>
  /* Under the header row of the pane (40px) that holds it. */
  .menu {
    position: absolute;
    top: 41px;
    left: 6px;
    min-width: 280px;
    max-width: calc(100% - 12px);
  }

  .list {
    max-height: min(320px, 50vh);
    overflow-y: auto;
    outline: none;
  }

  .item {
    display: flex;
    align-items: center;
  }

  .option {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 8px;
    height: 28px;
    padding: 0 8px;
    border-radius: 6px;
    color: var(--text);
    white-space: nowrap;
    cursor: default;
    transition: background-color 0.12s;
  }

  .option.on {
    background: var(--panel-2);
  }

  /* The current choice is marked by a dot in a column of its own. */
  .marks .option::before {
    content: '';
    flex: none;
    align-self: center;
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .marks .option[aria-selected='true']::before {
    background: var(--accent);
  }

  .option > :global(.dim) {
    color: var(--dim);
  }

  .option > :global(.fit) {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .option > :global(.end) {
    margin-left: auto;
    flex: none;
  }

  .remove {
    flex: none;
    width: 22px;
    height: 22px;
    margin-left: 2px;
    padding: 0;
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--dim);
    opacity: 0;
    transition:
      opacity 0.12s,
      color 0.12s;
  }

  .item:hover .remove,
  .remove:focus-visible {
    opacity: 1;
  }

  .remove:hover {
    color: var(--danger);
  }

  .none {
    margin: 0;
    padding: 6px 8px;
    color: var(--dim);
  }
</style>
