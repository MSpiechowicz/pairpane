import { splitResult } from '$lib/find/results';
import type { Entry } from '$lib/ipc/bindings/Entry';

import type { Listing } from './listing.svelte';
import { type Range, span } from './scroll';
import type { PaneView } from './view.svelte';

/** The side of the square a thumbnail has to fit in, in pixels. */
export const THUMB_SIZE = 128;

/** The names of pictures the backend can make thumbnails of; a dot file (`.png`) has no extension. */
const PICTURE = /.\.(?:jpe?g|png|gif|webp|bmp|heic|heif)$/i;

/** One picture to ask the backend for: where it is, and the key under which its thumbnail is kept. */
export interface ThumbTarget {
  key: string;
  /** The real folder that holds the picture, and its name in it. */
  folder: string;
  name: string;
}

/**
 * What to ask for to show `entry` of `listing` as a thumbnail, or `undefined` when it gets its icon: a name that does
 * not look like a picture, a folder, and anything inside an archive (reading it would unpack it). A search result or
 * a flat view lists items by their path below the folder searched, so the real folder is the one that holds the item.
 * The key changes with the file (its size and time), so a changed picture is asked for again.
 */
export function targetOf(listing: Listing, entry: Entry | undefined, size: number): ThumbTarget | undefined {
  if (entry === undefined || entry.kind !== 'file' || !PICTURE.test(entry.name)) {
    return undefined;
  }

  const root = listing.folder;
  if (root === undefined || listing.readonly) {
    return undefined;
  }

  const { folder, name } = listing.results === undefined ? { folder: root, name: entry.name } : splitResult(root, entry.name);
  const key = [folder, name, size, entry.size, entry.modified ?? ''].join('\0');

  return { key, folder, name };
}

/**
 * What the items of `view` in the run `visible`, then those in `near` (which holds them), ask for: the order is the
 * order to fetch in.
 */
export function wantedBy(view: PaneView, visible: Range, near: Range, size: number): ThumbTarget[] {
  const outside = span(near.first, near.last).filter((row) => row < visible.first || row >= visible.last);
  const targets: ThumbTarget[] = [];

  for (const row of [...span(visible.first, visible.last), ...outside]) {
    const target = targetOf(view.listing, view.entryAt(row), size);
    if (target !== undefined) {
      targets.push(target);
    }
  }

  return targets;
}
