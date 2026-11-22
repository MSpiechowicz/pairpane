import type { Entry } from '$lib/ipc/bindings/Entry';
import { type ArchiveSpot, archiveFile, archiveName, isWritableArchive } from '$lib/panes/archive';
import type { PaneView } from '$lib/panes/view.svelte';

import type { FileAction } from './dialogs';

/**
 * What a dialog is opened for: items to copy or move into a folder, one item to rename or link to, a new item, or
 * the first part of a split file to join in a folder, archives (or items of one) to extract, or items to add to the
 * folder of an archive.
 */
export type OpRequest =
  | { kind: 'copy' | 'move'; from: string; names: string[]; to: string }
  /** The parts that go with `name` (its `.001`) are joined into a file in the folder `to`. */
  | { kind: 'combine'; from: string; name: string; to: string }
  | { kind: 'rename'; path: string; name: string; dir: boolean }
  | { kind: 'mkdir' | 'mkfile'; path: string }
  /** A link to `name` in `path`, made in the folder `to`. */
  | { kind: 'symlink' | 'hardlink'; path: string; name: string; dir: boolean; to: string }
  /**
   * Each of `archives` is extracted into the folder `to`: the `names` in the folder it names (the selection of an
   * archive pane), or, with no names, the whole archive (archives picked in a folder).
   */
  | { kind: 'extract'; archives: ArchiveSpot[]; names: string[]; to: string }
  /** The `names` in the folder `from` are added to the archive `archive`, into its folder `dir` (`''`: its root). */
  | { kind: 'add'; archive: string; dir: string; from: string; names: string[] };

/**
 * The kinds of dialog a key asks for. Adding to an archive is not one of them: it is what copying becomes when the
 * other pane shows a writable archive.
 */
export type DialogKind = Exclude<OpRequest['kind'], 'add'>;

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

/** What the card says when an action would change an archive's contents. */
export const READ_ONLY_NOTICE = 'Archives are read-only here — extract first (F5).';

/** What the card says when an action other than copying would write into a pane that shows an archive. */
export const INTO_ARCHIVE_NOTICE = 'Only copying works into an archive.';

/** What the card says when files would be added to an archive of a kind that cannot be written (RAR, a lone `.gz`). */
export const CANT_ADD_NOTICE = 'Files can’t be added to this kind of archive.';

/** What the card says when an archive's items would be extracted into another archive. */
export const EXTRACT_INTO_ARCHIVE_NOTICE = 'Can’t extract into an archive.';

/** The actions that write into the other pane's folder. */
const WRITES_TO_OTHER: ReadonlySet<FileAction> = new Set(['copy', 'move', 'symlink', 'hardlink', 'split', 'combine', 'extract']);

/**
 * Why `action` cannot run between these panes, or `undefined` when it can: nothing changes an archive (copying out
 * of one is extracting), and only copying from a folder is written into one, when it is of a kind that can be.
 */
export function refusal(action: FileAction, source: PaneView, other: PaneView): string | undefined {
  if (source.listing.readonly && action !== 'copy' && action !== 'extract') {
    return READ_ONLY_NOTICE;
  }

  const target = other.listing.archive;
  if (target === undefined || !WRITES_TO_OTHER.has(action)) {
    return undefined;
  }

  if (source.listing.readonly || action === 'extract') {
    return EXTRACT_INTO_ARCHIVE_NOTICE;
  }

  if (action !== 'copy') {
    return INTO_ARCHIVE_NOTICE;
  }

  return isWritableArchive(archiveName(target.archive)) ? undefined : CANT_ADD_NOTICE;
}

/** The first part of a split file: the one the parts are joined from. */
export const isFirstPart = (name: string) => name.endsWith('.001');

/**
 * The dialog for a key in `source`'s pane, or `undefined` when there is nothing to act on. Copy and move offer the
 * folder of the other pane; rename and the links act on the entry under the cursor, a link goes to the other pane's
 * folder; so does combining, which acts on a first part (`.001`) only; a new folder or file is made in the pane's
 * folder. In an archive pane copying is extracting the selection, as it is for Extract in any pane (which takes the
 * archives that are selected in a folder). Copying into a pane that shows an archive adds the items to it.
 */
export function requestFor(kind: DialogKind, source: PaneView, other: PaneView): OpRequest | undefined {
  if (kind === 'extract' || (kind === 'copy' && source.listing.readonly)) {
    return extractRequest(source, other.listing.path ?? '');
  }

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
  if (names.length === 0) {
    return undefined;
  }

  const archive = other.listing.archive;
  if (kind === 'copy' && archive !== undefined) {
    return { kind: 'add', archive: archive.archive, dir: archive.dir, from: path, names };
  }

  return { kind, from: path, names, to: other.listing.path ?? '' };
}

/** The archives to extract from `source`: the open archive's selection, or the archives chosen in its folder. */
function extractRequest(source: PaneView, to: string): OpRequest | undefined {
  const { archive, path } = source.listing;
  if (archive !== undefined) {
    const names = targets(source);
    return names.length > 0 ? { kind: 'extract', archives: [archive], names, to } : undefined;
  }

  const archives = path === undefined ? [] : chosen(source).flatMap((entry) => archiveFile(path, entry) ?? []);
  return archives.length > 0 ? { kind: 'extract', archives: archives.map((archive) => ({ archive, dir: '' })), names: [], to } : undefined;
}

/** How much of a name to select for renaming: all of it but the extension (a folder or `.bashrc` has none). */
export function stemLength(name: string, dir: boolean): number {
  const dot = name.lastIndexOf('.');
  return dir || dot <= 0 || dot === name.length - 1 ? name.length : dot;
}
