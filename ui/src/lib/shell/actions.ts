import type { FileAction } from '$lib/ops/dialogs';
import type { PaneTabs } from '$lib/panes/tabs.svelte';

import type { Menu } from './menu';
import type { Action } from './shortcuts';

type Side = 'left' | 'right';

/** What the actions of the keymap act on: the shell's parts, as the app holds them. */
export interface ActionContext {
  /** The active pane's tabs, and which pane that is. */
  tabs: () => PaneTabs;
  active: () => Side;
  showMenu: (side: Side, kind: Menu) => void;
  files: (action: FileAction) => void;
  toggleTransfers: () => void;
  view: () => void;
  edit: () => void;
  compareFiles: () => void;
  quickView: () => void;
  find: () => void;
  flatView: () => void;
  terminal: () => void;
  palette: () => void;
}

/** Runs `action` (a shortcut, or the palette's choice): tabs, history and menus act on the active pane. */
export function runAction(action: Action, context: ActionContext) {
  const tabs = context.tabs();

  switch (action) {
    case 'newTab':
      tabs.add();
      break;
    case 'closeTab':
      tabs.close();
      break;
    case 'nextTab':
      tabs.cycle(1);
      break;
    case 'previousTab':
      tabs.cycle(-1);
      break;
    case 'back':
      tabs.back();
      break;
    case 'forward':
      tabs.forward();
      break;
    case 'driveMenuLeft':
      context.showMenu('left', 'drives');
      break;
    case 'driveMenuRight':
      context.showMenu('right', 'drives');
      break;
    case 'hotlist':
      context.showMenu(context.active(), 'hotlist');
      break;
    case 'transfers':
      context.toggleTransfers();
      break;
    case 'view':
      context.view();
      break;
    case 'edit':
      context.edit();
      break;
    case 'compareFiles':
      context.compareFiles();
      break;
    case 'quickView':
      context.quickView();
      break;
    case 'find':
      context.find();
      break;
    case 'flatView':
      context.flatView();
      break;
    case 'terminal':
      context.terminal();
      break;
    case 'palette':
      context.palette();
      break;
    default:
      context.files(action);
  }
}
