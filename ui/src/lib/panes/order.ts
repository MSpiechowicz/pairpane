import type { Entry } from '$lib/ipc/bindings/Entry';
import type { SortColumn as SavedColumn } from '$lib/ipc/bindings/SortColumn';

import { splitName } from './format';

/** The columns whose values the listing does not have: they are asked for the rows in view, one at a time, in batches or a folder at a time. */
export type ExtraColumn = 'dimensions' | 'checksum' | 'git';

/** What a pane can be sorted by: the columns of a listing, and the extra ones. */
export type SortColumn = SavedColumn | ExtraColumn;

export function isExtra(column: SortColumn): column is ExtraColumn {
  return column === 'dimensions' || column === 'checksum' || column === 'git';
}

/**
 * The sort of a tab as it is saved. The extra columns are not: what they sort by is only known for the rows that were
 * in view, so after a restart the tab is sorted by name.
 */
export function savedSort(column: SortColumn, desc: boolean): { sort: SavedColumn; desc: boolean } {
  return isExtra(column) ? { sort: 'name', desc: false } : { sort: column, desc };
}

export interface ViewOptions {
  column: SortColumn;
  desc: boolean;
  /** Case-insensitive "contains" on the name; empty shows everything. */
  filter: string;
  /** The computed size of a folder, when known. */
  sizeOf: (entry: Entry) => number | undefined;
  /** The value of an extra column for an entry (a number, or text for a checksum); `undefined` until it is known. */
  extraOf?: (entry: Entry, column: ExtraColumn) => number | string | undefined;
}

/** The sort key of every entry, indexed like `entries`. Numbers, lower-case extensions or checksums; `undefined` is unknown. */
function keysFor(entries: readonly (Entry | undefined)[], options: ViewOptions): (number | string | undefined)[] {
  const keys: (number | string | undefined)[] = new Array(entries.length);

  for (let i = 0; i < entries.length; i += 1) {
    const entry = entries[i];
    if (entry === undefined) {
      keys[i] = 0;
    } else if (isExtra(options.column)) {
      keys[i] = options.extraOf?.(entry, options.column);
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
 * not loaded yet come last, and a filter hides them. Entries whose value in an extra column is not known come last
 * of their kind, whichever way the sort runs.
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
      const ka = keys[a] as number | string;
      const kb = keys[b] as number | string;
      if (ka < kb) {
        return -sign;
      }
      if (ka > kb) {
        return sign;
      }

      return a - b;
    };

    const sorted = (run: number[]) => {
      const known = run.filter((i) => keys[i] !== undefined);
      return known.sort(compare).concat(run.filter((i) => keys[i] === undefined));
    };

    return sorted(dirs).concat(sorted(files), unloaded);
  }

  return dirs.concat(files, unloaded);
}
