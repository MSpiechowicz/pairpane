import type { Entry } from '$lib/ipc/bindings/Entry';
import type { PaneView } from '$lib/panes/view.svelte';

/**
 * What a dialog is opened for: items to copy or move into a folder, one item to rename or link to, a new item, or
 * the first part of a split file to join in a folder.
 */
export type OpRequest =
  | { kind: 'copy' | 'move'; from: string; names: string[]; to: string }
  /** The parts that go with `name` (its `.001`) are joined into a file in the folder `to`. */
  | { kind: 'combine'; from: string; name: string; to: string }
  | { kind: 'rename'; path: string; name: string; dir: boolean }
  | { kind: 'mkdir' | 'mkfile'; path: string }
  /** A link to `name` in `path`, made in the folder `to`. */
  | { kind: 'symlink' | 'hardlink'; path: string; name: string; dir: boolean; to: string };

/** The kinds of dialog a request can be for. */
export type DialogKind = OpRequest['kind'];

/**
 * The names an operation acts on: the selected entries the pane shows (the quick filter hides the rest, which an
 * operation must not touch unseen), or the entry under the cursor when none is. The `..` row is no entry, so it
 * never counts. Names are in the folder's order.
 */
export function targets(view: PaneView): string[] {
  return chosen(view).map((entry) => entry.name);
}

/** The entries {@link targets} names: the selected ones the pane shows, or the one under the cursor. */
export function chosen(view: PaneView): Entry[] {
  const selected = view.selectedShown();
  if (selected.length > 0) {
    return selected;
  }

  const under = view.entryAt(view.cursor);
  return under === undefined ? [] : [under];
}

/** The first part of a split file: the one the parts are joined from. */
export const isFirstPart = (name: string) => name.endsWith('.001');

/**
 * The dialog for a key in `source`'s pane, or `undefined` when there is nothing to act on. Copy and move offer the
 * folder of the other pane; rename and the links act on the entry under the cursor, a link goes to the other pane's
 * folder; so does combining, which acts on a first part (`.001`) only; a new folder or file is made in the pane's
 * folder.
 */
export function requestFor(kind: DialogKind, source: PaneView, other: PaneView): OpRequest | undefined {
  const path = source.listing.path;
  if (path === undefined) {
    return undefined;
  }

  if (kind === 'mkdir' || kind === 'mkfile') {
    return { kind, path };
  }

  if (kind === 'combine') {
    const part = source.entryAt(source.cursor);
    const to = other.listing.path ?? '';
    return part !== undefined && part.kind === 'file' && isFirstPart(part.name) ? { kind, from: path, name: part.name, to } : undefined;
  }

  if (kind === 'rename' || kind === 'symlink' || kind === 'hardlink') {
    const under = source.entryAt(source.cursor);
    const to = other.listing.path;
    if (under === undefined || (kind !== 'rename' && to === undefined)) {
      return undefined;
    }

    const dir = under.kind === 'dir';
    return kind === 'rename' ? { kind, path, name: under.name, dir } : { kind, path, name: under.name, dir, to: to! };
  }

  const names = targets(source);
  return names.length > 0 ? { kind, from: path, names, to: other.listing.path ?? '' } : undefined;
}

/** How much of a name to select for renaming: all of it but the extension (a folder or `.bashrc` has none). */
export function stemLength(name: string, dir: boolean): number {
  const dot = name.lastIndexOf('.');
  return dir || dot <= 0 || dot === name.length - 1 ? name.length : dot;
}
