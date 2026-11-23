import type { FilesChoice } from '$lib/shell/menu';
import { type Action, BINDINGS, keysOf } from '$lib/shell/shortcuts';

/** A command the palette lists: an action of the keymap, a choice of the Files menu, or one of the top bar's. */
export type CommandId = Exclude<Action, 'palette'> | FilesChoice | 'toggleHidden' | 'editSettings' | 'checkUpdates';

/** Every command, by the name the palette shows. */
export const COMMANDS: readonly { id: CommandId; label: string }[] = [
  { id: 'copy', label: 'Copy' },
  { id: 'move', label: 'Move' },
  { id: 'renameInline', label: 'Rename' },
  { id: 'rename', label: 'Rename or move…' },
  { id: 'multiRename', label: 'Multi-rename…' },
  { id: 'newFolder', label: 'New folder' },
  { id: 'newFile', label: 'New file' },
  { id: 'trash', label: 'Delete (to the Trash)' },
  { id: 'deletePermanently', label: 'Delete permanently' },
  { id: 'view', label: 'View file' },
  { id: 'edit', label: 'Edit file' },
  { id: 'editBuiltin', label: 'Edit in built-in editor' },
  { id: 'quickView', label: 'Quick view' },
  { id: 'properties', label: 'Properties' },
  { id: 'attributes', label: 'Change attributes…' },
  { id: 'symlink', label: 'Create symbolic link…' },
  { id: 'hardlink', label: 'Create hard link…' },
  { id: 'checksum', label: 'Create checksum file…' },
  { id: 'verifyChecksum', label: 'Verify checksum file' },
  { id: 'split', label: 'Split file…' },
  { id: 'combine', label: 'Combine files…' },
  { id: 'pack', label: 'Pack…' },
  { id: 'extract', label: 'Extract…' },
  { id: 'find', label: 'Find files…' },
  { id: 'compareFiles', label: 'Compare files…' },
  { id: 'compare', label: 'Compare folders' },
  { id: 'synchronize', label: 'Synchronize folders…' },
  { id: 'newTab', label: 'New tab' },
  { id: 'closeTab', label: 'Close tab' },
  { id: 'nextTab', label: 'Next tab' },
  { id: 'previousTab', label: 'Previous tab' },
  { id: 'back', label: 'Go back' },
  { id: 'forward', label: 'Go forward' },
  { id: 'driveMenuLeft', label: 'Drives of the left pane' },
  { id: 'driveMenuRight', label: 'Drives of the right pane' },
  { id: 'hotlist', label: 'Hotlist' },
  { id: 'transfers', label: 'Transfers' },
  { id: 'toggleHidden', label: 'Show or hide hidden files' },
  { id: 'editSettings', label: 'Edit settings' },
  { id: 'checkUpdates', label: 'Check for updates' },
];

/** Whether `id` is an action of the keymap. */
export function isAction(id: string): id is Action {
  return BINDINGS.some((binding) => binding.action === id);
}

/** The keys of the command `id`, as they are written, when it has any. */
export function keysFor(id: string): string | undefined {
  return isAction(id) ? keysOf(id) : undefined;
}
