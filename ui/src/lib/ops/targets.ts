import type { OpKind } from '$lib/ipc/bindings/OpKind';
import type { PaneView } from '$lib/panes/view.svelte';

/** What a dialog is opened for: items to copy or move into a folder, or one item to rename where it is. */
export type OpRequest =
  | { kind: OpKind; from: string; names: string[]; to: string }
  | { kind: 'rename'; path: string; name: string; dir: boolean };

/**
 * The names an operation acts on: the selected entries the pane shows (the quick filter hides the rest, which an
 * operation must not touch unseen), or the entry under the cursor when none is. The `..` row is no entry, so it
 * never counts. Names are in the folder's order.
 */
export function targets(view: PaneView): string[] {
  const names = view.selectedShown().map((entry) => entry.name);
  if (names.length > 0) {
    return names;
  }

  const under = view.entryAt(view.cursor);
  return under === undefined ? [] : [under.name];
}

/**
 * The dialog for a key in `source`'s pane, or `undefined` when there is nothing to act on. Copy and move offer the
 * folder of the other pane; rename always acts on the entry under the cursor.
 */
export function requestFor(kind: OpKind | 'rename', source: PaneView, other: PaneView): OpRequest | undefined {
  const path = source.listing.path;
  if (path === undefined) {
    return undefined;
  }

  if (kind === 'rename') {
    const under = source.entryAt(source.cursor);
    if (under === undefined) {
      return undefined;
    }

    return { kind, path, name: under.name, dir: under.kind === 'dir' };
  }

  const names = targets(source);
  return names.length > 0 ? { kind, from: path, names, to: other.listing.path ?? '' } : undefined;
}

/** How much of a name to select for renaming: all of it but the extension (a folder or `.bashrc` has none). */
export function stemLength(name: string, dir: boolean): number {
  const dot = name.lastIndexOf('.');
  return dir || dot <= 0 || dot === name.length - 1 ? name.length : dot;
}
