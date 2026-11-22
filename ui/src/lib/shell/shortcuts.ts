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
  | 'quickView';

/** Ctrl + key. (Ctrl on every OS: on macOS Cmd+W already closes the window and Cmd+Q quits the app.) */
function withCtrl(event: KeyboardEvent): Action | undefined {
  if (event.key === 'Tab') {
    return event.shiftKey ? 'previousTab' : 'nextTab';
  }

  if (event.shiftKey) {
    return undefined;
  }

  switch (event.key.toLowerCase()) {
    case 't':
      return 'newTab';
    case 'w':
      return 'closeTab';
    case 'd':
      return 'hotlist';
    case 'j':
      return 'transfers';
    case 'm':
      return 'multiRename';
    case 'q':
      return 'quickView';
    default:
      return undefined;
  }
}

/** Alt + key. */
function withAlt(event: KeyboardEvent): Action | undefined {
  if (event.shiftKey) {
    return undefined;
  }

  switch (event.key) {
    case 'ArrowLeft':
      return 'back';
    case 'ArrowRight':
      return 'forward';
    case 'F1':
      return 'driveMenuLeft';
    case 'F2':
      return 'driveMenuRight';
    case 'Enter':
      return 'properties';
    case 'F5':
      return 'pack';
    case 'F7':
      return 'find';
    case 'F9':
      return 'extract';
    default:
      return undefined;
  }
}

/**
 * A function key on its own, or Delete: F3 views the file, F4 edits it, F5 copies, F6 moves, Shift+F6 renames, F2 renames in place, Shift+F2
 * compares two files, F7 makes a folder, Shift+F4 a file, F8 and Delete move to the Trash, Shift+F8 and Shift+Delete delete for good.
 */
function functionKey(event: KeyboardEvent): Action | undefined {
  switch (event.key) {
    case 'F2':
      return event.shiftKey ? 'compareFiles' : 'renameInline';
    case 'F3':
      return event.shiftKey ? undefined : 'view';
    case 'F4':
      return event.shiftKey ? 'newFile' : 'edit';
    case 'F5':
      return event.shiftKey ? undefined : 'copy';
    case 'F6':
      return event.shiftKey ? 'rename' : 'move';
    case 'F7':
      return event.shiftKey ? undefined : 'newFolder';
    case 'F8':
    case 'Delete':
      return event.shiftKey ? 'deletePermanently' : 'trash';
    default:
      return undefined;
  }
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

  if (event.ctrlKey) {
    return withCtrl(event);
  }

  if (event.altKey) {
    return withAlt(event);
  }

  return functionKey(event);
}

/** Whether the key press goes to a text field: shortcuts and pane switching leave those alone. */
export function isTyping(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest('input, textarea') !== null;
}
