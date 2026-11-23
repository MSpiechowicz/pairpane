/**
 * What a window-level shortcut asks for. The drive menus, the hotlist, the file dialogs and the transfers panel
 * are run by the shell that owns them.
 */
export const ACTIONS = [
  'newTab',
  'closeTab',
  'nextTab',
  'previousTab',
  'back',
  'forward',
  'driveMenuLeft',
  'driveMenuRight',
  'hotlist',
  'copy',
  'move',
  'rename',
  'renameInline',
  'multiRename',
  'newFolder',
  'newFile',
  'trash',
  'deletePermanently',
  'properties',
  'extract',
  'pack',
  'find',
  'flatView',
  'layoutBrief',
  'layoutDetailed',
  'transfers',
  'view',
  'edit',
  'compareFiles',
  'quickView',
  'terminal',
  'palette',
] as const;

/** An action of the keymap. */
export type Action = (typeof ACTIONS)[number];

/** A key with the modifiers that must be held with it (and no other). */
export interface Chord {
  /** `KeyboardEvent.key`; a letter is matched whatever its case (caps lock). */
  key: string;
  ctrl?: boolean;
  alt?: boolean;
  shift?: boolean;
}

/** A key that stands for an action. */
export interface Binding extends Chord {
  action: Action;
}

/**
 * The keymap: Ctrl + a letter (Ctrl on every OS: on macOS Cmd+W already closes the window and Cmd+Q quits the app),
 * Alt + a key, and a function key or Delete on its own or with Shift. F3 views the file, F4 edits it, F5 copies, F6
 * moves, Shift+F6 renames, F2 renames in place, Shift+F2 compares two files, F7 makes a folder, Shift+F4 a file, F8 and
 * Delete move to the Trash, Shift+F8 and Shift+Delete delete for good, Ctrl+B lists every file below the folder, Ctrl+F1
 * shows the pane in brief and Ctrl+F2 in detail. An action may have more than one key; the first is the one shown.
 * These are the defaults: the user may set others (see `keymap`).
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
  { action: 'flatView', key: 'b', ctrl: true },
  { action: 'layoutBrief', key: 'F1', ctrl: true },
  { action: 'layoutDetailed', key: 'F2', ctrl: true },
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
function matches(event: KeyboardEvent, binding: Chord): boolean {
  const key = binding.key.length === 1 ? event.key.toLowerCase() === binding.key : event.key === binding.key;

  return key && event.ctrlKey === !!binding.ctrl && event.altKey === !!binding.alt && event.shiftKey === !!binding.shift;
}

/** How the keys of a chord are written: `Ctrl+K`, `Alt+Left`, `Shift+F2`. */
export function written({ key, ctrl, alt, shift }: Chord): string {
  const names: Record<string, string> = { ArrowLeft: 'Left', ArrowRight: 'Right', Delete: 'Del' };
  const name = names[key] ?? (key.length === 1 ? key.toUpperCase() : key);

  return [ctrl ? 'Ctrl' : '', alt ? 'Alt' : '', shift ? 'Shift' : '', name].filter((part) => part !== '').join('+');
}

/** The keys of `action` as they are written, when it has any (the first of its bindings in `bindings`). */
export function keysOf(action: Action, bindings: readonly Binding[] = BINDINGS): string | undefined {
  const binding = bindings.find((candidate) => candidate.action === action);

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
export function shortcutFor(event: KeyboardEvent, bindings: readonly Binding[] = BINDINGS): Action | undefined {
  if (event.isComposing || event.metaKey || (event.ctrlKey && event.altKey)) {
    return undefined;
  }

  return bindings.find((binding) => matches(event, binding))?.action;
}

/** The chord a key press is, for one to be set; `undefined` for a modifier pressed on its own, Cmd and AltGr. */
export function chordOf(event: KeyboardEvent): Chord | undefined {
  const modifierOnly = ['Control', 'Alt', 'Shift', 'Meta', 'AltGraph', 'CapsLock', 'Dead', 'Unidentified'].includes(event.key);
  if (modifierOnly || event.isComposing || event.metaKey || (event.ctrlKey && event.altKey)) {
    return undefined;
  }

  const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
  return { key, ctrl: event.ctrlKey || undefined, alt: event.altKey || undefined, shift: event.shiftKey || undefined };
}

/**
 * Why `chord` cannot be an action's key, or `undefined` when it can: one with neither Ctrl nor Alt must be a function key
 * or Delete (any other key alone types, moves the cursor or works the dialogs), Escape never is one, and neither is a key
 * the panes take for themselves first.
 */
export function refusalOf(chord: Chord, mac: boolean = /Mac/.test(navigator.userAgent)): string | undefined {
  if (chord.key === 'Escape') {
    return 'Escape closes dialogs: it cannot be a shortcut.';
  }

  // What the panes take before any shortcut: Ctrl+A selects all, and Option with a character types it on macOS.
  if (chord.ctrl && !chord.alt && chord.key.toLowerCase() === 'a') {
    return 'Ctrl+A selects every item: it cannot be a shortcut.';
  }
  if (mac && chord.alt && !chord.ctrl && chord.key.length === 1) {
    return 'Option with a character types it on macOS: hold Ctrl instead.';
  }

  const free = /^F([1-9]|1[0-2])$/.test(chord.key) || chord.key === 'Delete';
  if (!chord.ctrl && !chord.alt && !free) {
    return 'Hold Ctrl or Alt with this key, or use a function key or Delete.';
  }

  return undefined;
}

/** Whether the key press goes to a text field: shortcuts and pane switching leave those alone. */
export function isTyping(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest('input, textarea') !== null;
}
