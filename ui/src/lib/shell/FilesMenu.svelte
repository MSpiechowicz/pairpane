<script lang="ts">
  import type { FilesChoice } from './menu';

  /** The menu's items; a key shown next to an item does the same without the menu. */
  const ITEMS: readonly { action: FilesChoice; label: string; keys?: string }[] = [
    { action: 'newFolder', label: 'New folder', keys: 'F7' },
    { action: 'newFile', label: 'New file', keys: 'Shift+F4' },
    { action: 'editBuiltin', label: 'Edit in built-in editor' },
    { action: 'renameInline', label: 'Rename', keys: 'F2' },
    { action: 'trash', label: 'Delete', keys: 'F8' },
    { action: 'symlink', label: 'Create symbolic link…' },
    { action: 'hardlink', label: 'Create hard link…' },
    { action: 'properties', label: 'Properties', keys: 'Alt+Enter' },
    { action: 'attributes', label: 'Change attributes…' },
    { action: 'checksum', label: 'Create checksum file…' },
    { action: 'verifyChecksum', label: 'Verify checksum file' },
    { action: 'split', label: 'Split file…' },
    { action: 'combine', label: 'Combine files…' },
    { action: 'extract', label: 'Extract…', keys: 'Alt+F9' },
  ];

  let { onChoose }: { onChoose: (action: FilesChoice) => void } = $props();
</script>

<div class="popup" role="menu" aria-label="Files">
  {#each ITEMS as { action, label, keys } (action)}
    <button class="popup-item" type="button" role="menuitem" aria-keyshortcuts={keys} onclick={() => onChoose(action)}>
      <span>{label}</span>
      {#if keys}
        <kbd aria-hidden="true">{keys}</kbd>
      {/if}
    </button>
  {/each}
</div>
