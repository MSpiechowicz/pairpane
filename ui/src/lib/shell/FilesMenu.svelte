<script lang="ts">
  import { keysFor } from '$lib/palette/commands';

  import type { FilesChoice } from './menu';

  /** The menu's items; a key shown next to an item (the keymap's) does the same without the menu. */
  const ITEMS: readonly { action: FilesChoice; label: string }[] = [
    { action: 'newFolder', label: 'New folder' },
    { action: 'newFile', label: 'New file' },
    { action: 'editBuiltin', label: 'Edit in built-in editor' },
    { action: 'renameInline', label: 'Rename' },
    { action: 'multiRename', label: 'Multi-rename…' },
    { action: 'trash', label: 'Delete' },
    { action: 'symlink', label: 'Create symbolic link…' },
    { action: 'hardlink', label: 'Create hard link…' },
    { action: 'properties', label: 'Properties' },
    { action: 'attributes', label: 'Change attributes…' },
    { action: 'checksum', label: 'Create checksum file…' },
    { action: 'verifyChecksum', label: 'Verify checksum file' },
    { action: 'split', label: 'Split file…' },
    { action: 'combine', label: 'Combine files…' },
    { action: 'find', label: 'Find files…' },
    { action: 'compareFiles', label: 'Compare files…' },
    { action: 'compare', label: 'Compare folders' },
    { action: 'synchronize', label: 'Synchronize folders…' },
    { action: 'pack', label: 'Pack…' },
    { action: 'extract', label: 'Extract…' },
  ];

  let { onChoose }: { onChoose: (action: FilesChoice) => void } = $props();
</script>

<div class="popup" role="menu" aria-label="Files">
  {#each ITEMS as { action, label } (action)}
    {@const keys = keysFor(action)}
    <button class="popup-item" type="button" role="menuitem" aria-keyshortcuts={keys} onclick={() => onChoose(action)}>
      <span>{label}</span>
      {#if keys}
        <kbd aria-hidden="true">{keys}</kbd>
      {/if}
    </button>
  {/each}
</div>
