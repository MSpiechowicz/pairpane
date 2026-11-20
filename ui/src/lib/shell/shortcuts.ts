/**
 * What a window-level shortcut asks for. The drive menus, the hotlist, the copy dialogs and the transfers panel
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
  | 'transfers';

/** Ctrl + key. (Ctrl on every OS: on macOS Cmd+W already closes the window.) */
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
    default:
      return undefined;
  }
}

/** A function key on its own: F5 copies, F6 moves, Shift+F6 renames. */
function functionKey(event: KeyboardEvent): Action | undefined {
  switch (event.key) {
    case 'F5':
      return event.shiftKey ? undefined : 'copy';
    case 'F6':
      return event.shiftKey ? 'rename' : 'move';
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
 * alone is not one either (it switches panes). The function keys are the only shortcuts without a modifier.
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
