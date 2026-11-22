import type { ArchiveSpot } from '$lib/ipc/bindings/ArchiveSpot';
import type { Entry } from '$lib/ipc/bindings/Entry';

export type { ArchiveSpot };

/** What makes a file name an archive a pane can open (matched case-insensitively, at the end of the name). */
const EXTENSIONS = [
  '.zip',
  '.jar',
  '.tar',
  '.tar.gz',
  '.tgz',
  '.tar.xz',
  '.txz',
  '.tar.zst',
  '.tzst',
  '.tar.bz2',
  '.tbz2',
  '.tbz',
  '.gz',
  '.7z',
  '.rar',
];

/** Archives that can only be read: a RAR (no writer) and a lone `.gz` (one compressed file, not a collection). */
const READ_ONLY_EXTENSIONS: readonly string[] = ['.gz', '.rar'];

/** The archives files can be added to: every kind a pane opens but those that are only read. */
const WRITABLE_EXTENSIONS = EXTENSIONS.filter((extension) => !READ_ONLY_EXTENSIONS.includes(extension));

/** Whether `name` ends in one of `extensions` (matched case-insensitively; a name that is only an extension does not). */
function endsWithAny(name: string, extensions: readonly string[]): boolean {
  const lower = name.toLowerCase();
  return extensions.some((extension) => lower.length > extension.length && lower.endsWith(extension));
}

/** Whether a file called `name` is an archive to open (it is not a name that is only an extension). */
export function isArchiveName(name: string): boolean {
  return endsWithAny(name, EXTENSIONS);
}

/** Whether files can be added to the archive called `name`: not a RAR, not a lone `.gz` (`a.tar.gz` is). */
export function isWritableArchive(name: string): boolean {
  return endsWithAny(name, WRITABLE_EXTENSIONS);
}

/** The path of the entry `name` in the real folder `folder`, with the folder's own kind of separator. */
export function joinPath(folder: string, name: string): string {
  if (/[\\/]$/.test(folder)) {
    return folder + name;
  }

  return folder + (folder.includes('\\') && !folder.includes('/') ? '\\' : '/') + name;
}

/** The last name of an archive's path. */
export function archiveName(archive: string): string {
  return archive.split(/[\\/]/).at(-1) ?? archive;
}

/** The folder `name` inside the archive folder `dir` (`""` is the archive's root). */
export function joinInner(dir: string, name: string): string {
  return dir === '' ? name : `${dir}/${name}`;
}

/**
 * Where going up from the archive folder `archive` leads, and the name to put the cursor on there: the folder
 * above in the archive (the one we leave), or from its root the real `folder` that holds it (the archive itself).
 */
export function above(
  archive: ArchiveSpot,
  folder: string,
): { spot: ArchiveSpot; left: string } | { folder: string; left: string } {
  if (archive.dir === '') {
    return { folder, left: archiveName(archive.archive) };
  }

  const cut = archive.dir.lastIndexOf('/');
  return {
    spot: {
      archive: archive.archive,
      dir: archive.dir.slice(0, Math.max(cut, 0)),
    },
    left: archive.dir.slice(cut + 1),
  };
}

/** The path of the file `entry` when it is an archive to open from the real folder `folder`. */
export function archiveFile(folder: string, entry: Entry | undefined): string | undefined {
  return entry?.kind === 'file' && isArchiveName(entry.name) ? joinPath(folder, entry.name) : undefined;
}

/** Whether both are the same place in the same archive. */
export function sameSpot(a: ArchiveSpot | undefined, b: ArchiveSpot | undefined): boolean {
  if (a === undefined || b === undefined) {
    return a === b;
  }

  return a.archive === b.archive && a.dir === b.dir;
}

/** The key of a crumb in the path bar: real folders are told apart by their path, archive steps by their folder too. */
export const crumbKey = (crumb: { path: string; inside?: string }) => `${crumb.path}\0${crumb.inside ?? ''}`;
