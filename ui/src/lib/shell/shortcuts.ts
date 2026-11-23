/**
 * What a window-level shortcut asks for. The drive menus, the hotlist, the file dialogs and the transfers panel
 * are run by the shell that owns them.
 */
export type Action =
  | 'newTab'
  | 'closeTab'
  | 'nextTab'
  | 'previousTab'
  | 'back'
  | 'forward'
  | 'driveMenuLeft'
  | 'driveMenuRight'
  | 'hotlist'
  | 'copy'
  | 'move'
  | 'rename'
  | 'renameInline'
  | 'multiRename'
  | 'newFolder'
  | 'newFile'
  | 'trash'
  | 'deletePermanently'
  | 'properties'
  | 'extract'
  | 'pack'
  | 'find'
  | 'transfers'
  | 'view'
  | 'edit'
  | 'compareFiles'
  | 'quickView'
  | 'palette';

/** A key that stands for an action, with the modifiers that must be held (and no other). */
export interface Binding {
  action: Action;
  /** `KeyboardEvent.key`; a letter is matched whatever its case (caps lock). */
  key: string;
  ctrl?: boolean;
  alt?: boolean;
  shift?: boolean;
}

/**
 * The keymap: Ctrl + a letter (Ctrl on every OS: on macOS Cmd+W already closes the window and Cmd+Q quits the app),
 * Alt + a key, and a function key or Delete on its own or with Shift. F3 views the file, F4 edits it, F5 copies, F6
 * moves, Shift+F6 renames, F2 renames in place, Shift+F2 compares two files, F7 makes a folder, Shift+F4 a file, F8 and
 * Delete move to the Trash, Shift+F8 and Shift+Delete delete for good. An action may have more than one key; the first
 * is the one shown.
 */
export const BINDINGS: readonly Binding[] = [
  { action: 'palette', key: 'k', ctrl: true },
  { action: 'newTab', key: 't', ctrl: true },
  { action: 'closeTab', key: 'w', ctrl: true },
  { action: 'nextTab', key: 'Tab', ctrl: true },
  { action: 'previousTab', key: 'Tab', ctrl: true, shift: true },
  { action: 'hotlist', key: 'd', ctrl: true },
  { action: 'transfers', key: 'j', ctrl: true },
  { action: 'multiRename', key: 'm', ctrl: true },
  { action: 'quickView', key: 'q', ctrl: true },
  { action: 'back', key: 'ArrowLeft', alt: true },
  { action: 'forward', key: 'ArrowRight', alt: true },
  { action: 'driveMenuLeft', key: 'F1', alt: true },
  { action: 'driveMenuRight', key: 'F2', alt: true },
  { action: 'properties', key: 'Enter', alt: true },
  { action: 'pack', key: 'F5', alt: true },
  { action: 'find', key: 'F7', alt: true },
  { action: 'extract', key: 'F9', alt: true },
  { action: 'renameInline', key: 'F2' },
  { action: 'compareFiles', key: 'F2', shift: true },
  { action: 'view', key: 'F3' },
  { action: 'edit', key: 'F4' },
  { action: 'newFile', key: 'F4', shift: true },
  { action: 'copy', key: 'F5' },
  { action: 'move', key: 'F6' },
  { action: 'rename', key: 'F6', shift: true },
  { action: 'newFolder', key: 'F7' },
  { action: 'trash', key: 'F8' },
  { action: 'trash', key: 'Delete' },
  { action: 'deletePermanently', key: 'F8', shift: true },
  { action: 'deletePermanently', key: 'Delete', shift: true },
];

/** Whether `event` is the key of `binding`, with exactly its modifiers. */
function matches(event: KeyboardEvent, binding: Binding): boolean {
  const key = binding.key.length === 1 ? event.key.toLowerCase() === binding.key : event.key === binding.key;

  return key && event.ctrlKey === !!binding.ctrl && event.altKey === !!binding.alt && event.shiftKey === !!binding.shift;
}

/** How the keys of a binding are written: `Ctrl+K`, `Alt+Left`, `Shift+F2`. */
function written({ key, ctrl, alt, shift }: Binding): string {
  const names: Record<string, string> = { ArrowLeft: 'Left', ArrowRight: 'Right', Delete: 'Del' };
  const name = names[key] ?? (key.length === 1 ? key.toUpperCase() : key);

  return [ctrl ? 'Ctrl' : '', alt ? 'Alt' : '', shift ? 'Shift' : '', name].filter((part) => part !== '').join('+');
}

/** The keys of `action` as they are written, when it has any (the first of its bindings). */
export function keysOf(action: Action): string | undefined {
  const binding = BINDINGS.find((candidate) => candidate.action === action);

  return binding === undefined ? undefined : written(binding);
}

/** F5 reloads the webview, which would lose every pane and job on screen: it is never left to the webview. */
export function reloadsPage(event: KeyboardEvent): boolean {
  return event.key === 'F5';
}

/**
 * The action a key press stands for. Ctrl+Alt (AltGr, which types characters) and Cmd are never shortcuts; Tab
 * alone is not one either (it switches panes). The function keys and Delete are the only shortcuts without a modifier.
 */
export function shortcutFor(event: KeyboardEvent): Action | undefined {
  if (event.isComposing || event.metaKey || (event.ctrlKey && event.altKey)) {
    return undefined;
  }

  return BINDINGS.find((binding) => matches(event, binding))?.action;
}

/** Whether the key press goes to a text field: shortcuts and pane switching leave those alone. */
export function isTyping(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest('input, textarea') !== null;
}
