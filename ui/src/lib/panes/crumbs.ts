import type { Crumb } from '$lib/ipc/bindings/Crumb';
import type { Volume } from '$lib/ipc/bindings/Volume';

/** The name of the filesystem root when no volume claims it: "System" for `/`, the drive letter for `C:\`. */
function rootName(path: string): string {
  const drive = path.replace(/[\\/]+$/, '');
  return drive === '' ? 'System' : drive;
}

/** A root crumb (`/`, `C:\`) is its own path with a trailing separator. */
const isRoot = (crumb: Crumb) => /[\\/]$/.test(crumb.name);

/**
 * A path without its trailing separators, so `C:\Mounts\Data\` (how a folder-mounted volume is reported) and
 * `C:\Mounts\Data` (a crumb) are one path. A root (`/`, `C:\`) stays as it is.
 */
function trimmed(path: string): string {
  const bare = path.replace(/[\\/]+$/, '');
  return bare === '' || /^[A-Za-z]:$/.test(bare) ? path : bare;
}

/** Whether two paths are the same folder, whatever their trailing separators. */
export const samePath = (a: string, b: string) => trimmed(a) === trimmed(b);

/** The deepest crumb that is a volume's path, with that volume. */
function deepest(crumbs: readonly Crumb[], volumes: readonly Volume[]): { index: number; volume: Volume } | undefined {
  for (let index = crumbs.length - 1; index >= 0; index -= 1) {
    const volume = volumes.find((candidate) => samePath(candidate.path, crumbs[index].path));
    if (volume !== undefined) {
      return { index, volume };
    }
  }

  return undefined;
}

/** The volume a folder is in: the deepest one whose path is one of the folder's crumbs. */
export function volumeOf(crumbs: readonly Crumb[], volumes: readonly Volume[]): Volume | undefined {
  return deepest(crumbs, volumes)?.volume;
}

/**
 * The crumbs of a path as the header shows them: starting at the deepest volume that contains the folder, named
 * after it (`Home / Projects`, `USB / photos`). Without such a volume the path starts at the root, `System` or `C:`.
 * Volumes are matched by their path, trailing separators aside.
 */
export function anchored(crumbs: readonly Crumb[], volumes: readonly Volume[]): Crumb[] {
  const found = deepest(crumbs, volumes);
  if (found !== undefined) {
    return [{ name: found.volume.name, path: crumbs[found.index].path }, ...crumbs.slice(found.index + 1)];
  }

  if (crumbs.length > 0 && isRoot(crumbs[0])) {
    return [{ name: rootName(crumbs[0].name), path: crumbs[0].path }, ...crumbs.slice(1)];
  }

  return [...crumbs];
}

/** What a tab that is not the active one shows: its folder's name, or the volume's name at a volume's root. */
export function tabLabel(path: string | null, volumes: readonly Volume[]): string {
  if (path === null) {
    return 'Home';
  }

  const volume = volumes.find((candidate) => samePath(candidate.path, path));
  if (volume !== undefined) {
    return volume.name;
  }

  const parts = path.split(/[\\/]/).filter((part) => part !== '');
  if (parts.length === 1 && /^[A-Za-z]:$/.test(parts[0])) {
    return parts[0];
  }

  return parts.at(-1) ?? rootName(path);
}
