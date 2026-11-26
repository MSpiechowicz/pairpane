import type { Entry } from '$lib/ipc/bindings/Entry';
import type { PaneView } from '$lib/panes/view.svelte';
import { REMOTE_NOTICE } from '$lib/ops/targets';
import { fileTarget, type Target } from '$lib/viewer/viewer.svelte';

export const NEED_TWO_NOTICE = 'Select two files, or put the cursor on a file in each pane.';
export const QUICK_NOTICE = 'Close the quick view first.';
export const FOLDER_NOTICE = 'Folders cannot be compared by content.';
export const LINK_NOTICE = 'A link cannot be compared; compare the file it points to.';
export const OTHER_NOTICE = 'Only files can be compared.';
export const SAME_NOTICE = 'That is the same file twice; pick two different files.';

/** Whether the two targets are one file (the same entry of the same archive, too). */
const sameFile = (a: Target, b: Target) => a.path === b.path && a.name === b.name && a.inside === b.inside;

/** The two files of a comparison. */
export interface Picked {
  left: Target;
  right: Target;
}

export interface Panes {
  /** The pane that has the cursor, and whether it is the left or the right one. */
  active: PaneView;
  other: PaneView;
  side: 'left' | 'right';
  /** The other pane is hidden by the quick view. */
  quick: boolean;
}

/** Why `entry` is not a file that can be compared, or `undefined` when it is one. */
function refusal(entry: Entry | undefined, up: boolean): string | undefined {
  if (up || entry?.kind === 'dir') {
    return FOLDER_NOTICE;
  }

  if (entry === undefined) {
    return NEED_TWO_NOTICE;
  }

  if (entry.linkTarget !== null) {
    return LINK_NOTICE;
  }

  return entry.kind === 'file' ? undefined : OTHER_NOTICE;
}

/** The file under a pane's cursor, or why there is none. */
function underCursor(view: PaneView): Target | string {
  const entry = view.entryAt(view.cursor);
  const refused = refusal(entry, view.isUp(view.cursor));

  return refused ?? fileTarget(view, entry!);
}

/** The two selected files of the active pane, in the order the folder lists them. */
function selected(view: PaneView): Picked | string {
  const entries = view.selectedShown();
  if (entries.length !== 2) {
    return NEED_TWO_NOTICE;
  }

  const refused = entries.map((entry) => refusal(entry, false)).find((notice) => notice !== undefined);

  return refused ?? { left: fileTarget(view, entries[0]), right: fileTarget(view, entries[1]) };
}

/** The cursor file of each pane, the left pane's on the left. */
function cursors({ active, other, side }: Panes): Picked | string {
  const mine = underCursor(active);
  const theirs = underCursor(other);
  if (typeof mine === 'string') {
    return mine;
  }

  if (typeof theirs === 'string') {
    return theirs;
  }

  return side === 'left' ? { left: mine, right: theirs } : { left: theirs, right: mine };
}

/**
 * What Shift+F2 compares: exactly two selected files of the active pane, else (nothing selected) the file under the
 * cursor of each pane. A string is the refusal, in words for a card. A file inside an archive is compared too (read-only).
 */
export function pickFiles(panes: Panes): Picked | string {
  const { active, other, quick } = panes;
  if (active.listing.remote !== undefined || other.listing.remote !== undefined) {
    return REMOTE_NOTICE;
  }

  const count = active.selectedShown().length;

  if (count === 0 && quick) {
    return QUICK_NOTICE;
  }

  const picked = count === 0 ? cursors(panes) : selected(active);
  if (typeof picked !== 'string' && sameFile(picked.left, picked.right)) {
    return SAME_NOTICE;
  }

  return picked;
}
