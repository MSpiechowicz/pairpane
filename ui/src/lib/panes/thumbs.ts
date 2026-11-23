import { splitResult } from '$lib/find/results';
import type { Entry } from '$lib/ipc/bindings/Entry';

import type { Listing } from './listing.svelte';
import { inFetchOrder, type Range } from './scroll';
import type { PaneView } from './view.svelte';

/** The side of the square a thumbnail has to fit in, in pixels. */
export const THUMB_SIZE = 128;

/** The names of pictures the backend can make thumbnails of (and read the size of); a dot file (`.png`) has no extension. */
const PICTURE = /.\.(?:jpe?g|png|gif|webp|bmp|heic|heif)$/i;

/** Whether `name` looks like a picture. */
export function isPicture(name: string): boolean {
  return PICTURE.test(name);
}

/** One picture to ask the backend for: where it is, and the key under which its thumbnail is kept. */
export interface ThumbTarget {
  key: string;
  /** The real folder that holds the picture, and its name in it. */
  folder: string;
  name: string;
}

/**
 * Where the item `entry` of `listing` really is, for what the backend reads from disk: the real folder that holds it and
 * its name there. `undefined` for `..` and for anything inside an archive (reading it would unpack it). A search result
 * or a flat view lists items by their path below the folder searched, so the real folder is the one that holds the item.
 */
export function whereIs(listing: Listing, entry: Entry | undefined): { folder: string; name: string } | undefined {
  const root = listing.folder;
  if (entry === undefined || root === undefined || listing.readonly) {
    return undefined;
  }

  return listing.results === undefined ? { folder: root, name: entry.name } : splitResult(root, entry.name);
}

/**
 * Where the file `entry` of `listing` really is (see {@link whereIs}): `undefined` for anything that is not a file too.
 * The key says which file this is and what state it is in (its size and time), so that a changed file is not taken for
 * the one before.
 */
export function placeOf(listing: Listing, entry: Entry | undefined): { key: string; folder: string; name: string } | undefined {
  const at = entry?.kind === 'file' ? whereIs(listing, entry) : undefined;
  if (entry === undefined || at === undefined) {
    return undefined;
  }

  return { ...at, key: [at.folder, at.name, entry.size, entry.modified ?? ''].join('\0') };
}

/**
 * What to ask for to show `entry` of `listing` as a thumbnail, or `undefined` when it gets its icon: a name that does
 * not look like a picture, a folder, and anything inside an archive. The key changes with the file, so a changed
 * picture is asked for again.
 */
export function targetOf(listing: Listing, entry: Entry | undefined, size: number): ThumbTarget | undefined {
  const place = entry && isPicture(entry.name) ? placeOf(listing, entry) : undefined;
  return place && { ...place, key: `${place.key}\0${size}` };
}

/**
 * What the items of `view` in the run `visible`, then those in `near` (which holds them), ask for: the order is the
 * order to fetch in.
 */
export function wantedBy(view: PaneView, visible: Range, near: Range, size: number): ThumbTarget[] {
  const targets: ThumbTarget[] = [];

  for (const row of inFetchOrder(visible, near)) {
    const target = targetOf(view.listing, view.entryAt(row), size);
    if (target !== undefined) {
      targets.push(target);
    }
  }

  return targets;
}
