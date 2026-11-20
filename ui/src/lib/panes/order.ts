import type { Entry } from '$lib/ipc/bindings/Entry';

import { splitName } from './format';

export type SortColumn = 'name' | 'ext' | 'size' | 'modified';

export interface ViewOptions {
  column: SortColumn;
  desc: boolean;
  /** Case-insensitive "contains" on the name; empty shows everything. */
  filter: string;
  /** The computed size of a folder, when known. */
  sizeOf: (entry: Entry) => number | undefined;
}

/** The sort key of every entry, indexed like `entries`. Numbers or lower-case extensions. */
function keysFor(entries: readonly (Entry | undefined)[], options: ViewOptions): ArrayLike<number | string> {
  const keys: (number | string)[] = new Array(entries.length);

  for (let i = 0; i < entries.length; i += 1) {
    const entry = entries[i];
    if (entry === undefined) {
      keys[i] = 0;
    } else if (options.column === 'ext') {
      keys[i] = splitName(entry).ext.toLowerCase();
    } else if (options.column === 'size') {
      keys[i] = entry.kind === 'dir' ? (options.sizeOf(entry) ?? 0) : entry.size;
    } else {
      keys[i] = entry.modified ?? -1;
    }
  }

  return keys;
}

/**
 * The order the pane shows its entries in, as indexes into `entries`; `null` when that is the backend order
 * (name ascending, no filter). Folders stay on top. Ties keep the backend order, also when descending. Entries
 * not loaded yet come last, and a filter hides them.
 */
export function viewOrder(entries: readonly (Entry | undefined)[], options: ViewOptions): number[] | null {
  const filter = options.filter.toLowerCase();
  if (options.column === 'name' && !options.desc && filter === '') {
    return null;
  }

  const dirs: number[] = [];
  const files: number[] = [];
  const unloaded: number[] = [];

  for (let i = 0; i < entries.length; i += 1) {
    const entry = entries[i];
    if (entry === undefined) {
      if (filter === '') {
        unloaded.push(i);
      }
    } else if (filter === '' || entry.name.toLowerCase().includes(filter)) {
      (entry.kind === 'dir' ? dirs : files).push(i);
    }
  }

  if (options.column === 'name') {
    if (options.desc) {
      dirs.reverse();
      files.reverse();
    }
  } else {
    const keys = keysFor(entries, options);
    const sign = options.desc ? -1 : 1;
    const compare = (a: number, b: number) => {
      const ka = keys[a];
      const kb = keys[b];
      if (ka < kb) {
        return -sign;
      }
      if (ka > kb) {
        return sign;
      }

      return a - b;
    };

    dirs.sort(compare);
    files.sort(compare);
  }

  return dirs.concat(files, unloaded);
}
