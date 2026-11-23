<script lang="ts">
  import type { PaneLayout } from '$lib/ipc/bindings/PaneLayout';
  import { keysFor } from '$lib/palette/commands';

  import type { Action } from './shortcuts';

  /** The menu's items: how the active pane shows its folder; the key shown next to an item does the same without the menu. */
  const ITEMS: readonly { layout: PaneLayout; action: Action; label: string }[] = [
    { layout: 'detailed', action: 'layoutDetailed', label: 'Detailed' },
    { layout: 'brief', action: 'layoutBrief', label: 'Brief' },
  ];

  let { layout, onChoose }: { layout: PaneLayout; onChoose: (layout: PaneLayout) => void } = $props();
</script>

<div class="popup" role="menu" aria-label="View">
  {#each ITEMS as { layout: choice, action, label } (choice)}
    {@const keys = keysFor(action)}
    <button
      class="popup-item"
      class:on={layout === choice}
      type="button"
      role="menuitemradio"
      aria-checked={layout === choice}
      aria-keyshortcuts={keys}
      onclick={() => onChoose(choice)}
    >
      <span><span class="check" aria-hidden="true">{layout === choice ? '✓' : ''}</span>{label}</span>
      {#if keys}
        <kbd aria-hidden="true">{keys}</kbd>
      {/if}
    </button>
  {/each}
</div>

<style>
  .check {
    display: inline-block;
    width: 16px;
    color: var(--accent);
  }
</style>
