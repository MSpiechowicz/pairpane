import type { PaneView } from './view.svelte';

/** Whether this is macOS, where Option+key types a character (elsewhere Alt+key is left to menu accelerators). */
export const isMacPlatform = () => /Mac/.test(navigator.userAgent);

/** Whether this is Windows, where the Trash is called the Recycle Bin. */
export const isWindowsPlatform = () => /Windows/.test(navigator.userAgent);

/**
 * A key that types a character. AltGr (which Windows and Linux report as Ctrl+Alt) types one too, and so does
 * Option on macOS; Ctrl/Cmd shortcuts and a lone Alt elsewhere do not.
 */
function isPrintable(event: KeyboardEvent, mac: boolean): boolean {
  if (event.key.length !== 1 || event.metaKey) {
    return false;
  }

  if (event.ctrlKey) {
    return event.altKey;
  }

  return !event.altKey || mac;
}

function onPrintable(view: PaneView, key: string) {
  switch (key) {
    case ' ':
      view.space();
      break;
    case '+':
      view.beginMask('select');
      break;
    case '-':
      view.beginMask('deselect');
      break;
    case '*':
      view.invert();
      break;
    default:
      view.setFilter(view.filter + key);
  }
}

/**
 * The cursor one tile row up or down. It stays where it is when there is no row that way; moving down into a last row
 * that is not full puts it on the last tile.
 */
function moveByRow(view: PaneView, direction: -1 | 1, perRow: number) {
  const row = Math.floor(view.cursor / perRow) + direction;
  if (row < 0 || row > Math.floor((view.count - 1) / perRow)) {
    return;
  }

  view.moveCursor(direction * perRow);
}

/**
 * `pageSize` is the items the list shows at once: the rows in the detailed layout, the items of a column in the brief
 * one, the tiles in view in the thumbnails one, whose rows hold `perRow` tiles.
 */
function onNamed(view: PaneView, key: string, pageSize: number, perRow: number): boolean {
  const tiles = view.layout === 'thumbnails';

  switch (key) {
    case 'ArrowUp':
    case 'ArrowDown': {
      const direction = key === 'ArrowUp' ? -1 : 1;
      if (tiles) {
        moveByRow(view, direction, Math.max(1, perRow));
      } else {
        view.moveCursor(direction);
      }

      return true;
    }
    case 'PageUp':
      view.moveCursor(-Math.max(1, pageSize));
      return true;
    case 'PageDown':
      view.moveCursor(Math.max(1, pageSize));
      return true;
    case 'ArrowLeft':
    case 'ArrowRight': {
      // The tiles run along rows: one to the side is the next item. The brief layout runs its items down columns: a
      // column to the side is a column's worth of items away. In the detailed layout the keys are not the pane's.
      if (tiles) {
        view.moveCursor(key === 'ArrowLeft' ? -1 : 1);
        return true;
      }

      if (view.layout !== 'brief') {
        return false;
      }

      const column = Math.max(1, pageSize);
      view.moveCursor(key === 'ArrowLeft' ? -column : column);
      return true;
    }
    case 'Home':
      view.setCursor(0);
      return true;
    case 'End':
      view.setCursor(view.count - 1);
      return true;
    case 'Enter': {
      const opens = view.opens(view.cursor);
      if (opens) {
        void view.open(view.cursor);
      }

      return opens;
    }
    case 'Insert':
      view.insert();
      return true;
    case 'Backspace':
      if (view.filter === '') {
        void view.goUp();
      } else {
        view.setFilter(view.filter.slice(0, -1));
      }

      return true;
    case 'Escape':
      if (view.filter === '') {
        return false;
      }

      view.setFilter('');
      return true;
    default:
      return false;
  }
}

/**
 * Runs a key press on the pane's list; `true` when it was a pane key (the caller then prevents the default).
 * Keys typed into something else (the mask prompt), composition and Tab (it switches panes) are not ours. `perRow`
 * is the tiles in a row of the thumbnails layout.
 */
export function handleKey(
  view: PaneView,
  event: KeyboardEvent,
  pageSize: number,
  mac: boolean = isMacPlatform(),
  perRow = 1,
): boolean {
  if (event.isComposing || event.key === 'Tab' || event.target !== event.currentTarget) {
    return false;
  }

  if ((event.ctrlKey || event.metaKey) && !event.altKey && event.key.toLowerCase() === 'a') {
    view.selectAll();
    return true;
  }

  if (isPrintable(event, mac)) {
    onPrintable(view, event.key);
    return true;
  }

  if (event.ctrlKey || event.altKey || event.metaKey) {
    return false;
  }

  return onNamed(view, event.key, pageSize, perRow);
}
