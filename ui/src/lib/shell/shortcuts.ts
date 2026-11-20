/** What a window-level shortcut asks for. The drive menus and the hotlist are run by the shell that owns them. */
export type Action =
  | 'newTab'
  | 'closeTab'
  | 'nextTab'
  | 'previousTab'
  | 'back'
  | 'forward'
  | 'driveMenuLeft'
  | 'driveMenuRight'
  | 'hotlist';

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

/**
 * The action a key press stands for. Ctrl+Alt (AltGr, which types characters) and Cmd are never shortcuts; Tab
 * alone is not one either (it switches panes).
 */
export function shortcutFor(event: KeyboardEvent): Action | undefined {
  if (event.isComposing || event.metaKey || (event.ctrlKey && event.altKey)) {
    return undefined;
  }

  if (event.ctrlKey) {
    return withCtrl(event);
  }

  return event.altKey ? withAlt(event) : undefined;
}

/** Whether the key press goes to a text field: shortcuts and pane switching leave those alone. */
export function isTyping(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest('input, textarea') !== null;
}
