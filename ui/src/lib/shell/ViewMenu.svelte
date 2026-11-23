<script lang="ts">
  import type { Layout } from '$lib/panes/layout';
  import { keysFor } from '$lib/palette/commands';

  import type { Action } from './shortcuts';

  /** The menu's items: how the active pane shows its folder; the key shown next to an item does the same without the menu. */
  const ITEMS: readonly { layout: Layout; action: Action; label: string }[] = [
    { layout: 'detailed', action: 'layoutDetailed', label: 'Detailed' },
    { layout: 'brief', action: 'layoutBrief', label: 'Brief' },
    { layout: 'thumbnails', action: 'layoutThumbnails', label: 'Thumbnails' },
  ];

  let { layout, onChoose }: { layout: Layout; onChoose: (layout: Layout) => void } = $props();
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
