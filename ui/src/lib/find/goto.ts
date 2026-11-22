import type { ArchiveSpot } from '$lib/ipc/bindings/ArchiveSpot';
import type { FindHit } from '$lib/ipc/bindings/FindHit';
import { joinPath } from '$lib/panes/archive';

/** What "Go to" needs of the pane that goes: it opens a place, then puts its cursor on a name. */
export interface Navigation {
  listing: {
    open(path: string): Promise<void>;
    openArchive(spot: ArchiveSpot): Promise<void>;
  };
  pointAt(name: string): void;
}

/** The part of `path` (names joined with `/`) before its last name, and that name. */
function splitLast(path: string): { dir: string[]; name: string } {
  const names = path.split('/').filter((part) => part !== '');
  return { dir: names.slice(0, -1), name: names.at(-1) ?? '' };
}

/**
 * Opens the folder of a result in the pane, with the cursor on it. A result inside an archive opens the archive at the
 * folder of the entry, with the cursor on the entry. `root` is the folder that was searched.
 */
export async function goTo(pane: Navigation, root: string, hit: FindHit): Promise<void> {
  const found = splitLast(hit.path);

  if (hit.inner === null) {
    await pane.listing.open(found.dir.reduce(joinPath, root));
    pane.pointAt(found.name);
    return;
  }

  const archive = joinPath(found.dir.reduce(joinPath, root), found.name);
  const entry = splitLast(hit.inner);
  await pane.listing.openArchive({ archive, dir: entry.dir.join('/') });
  pane.pointAt(entry.name);
}
