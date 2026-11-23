import type { Dimensions } from '$lib/ipc/bindings/Dimensions';
import type { Entry } from '$lib/ipc/bindings/Entry';
import type { GitMark } from '$lib/ipc/bindings/GitMark';

import { displayName } from './format';
import type { Listing } from './listing.svelte';
import type { ExtraColumn } from './order';
import { inFetchOrder, type Range } from './scroll';
import { placeOf, whereIs } from './thumbs';
import type { PaneView } from './view.svelte';

/** How many hex digits of a checksum the column shows; the tooltip has all of them. */
export const CHECKSUM_DIGITS = 12;

/** The text shown where a value is known not to exist. */
const NONE = '–';

/**
 * The names of pictures the backend can read the size of: the ones it makes thumbnails of, and AVIF, which it reads the
 * header of but does not decode. A dot file (`.avif`) has no extension.
 */
const SIZED = /.\.(?:jpe?g|png|gif|webp|bmp|heic|heif|avif)$/i;

/** Whether `name` looks like a picture the backend can read the size of. */
export function hasDimensions(name: string): boolean {
  return SIZED.test(name);
}

/** One file to ask the backend about: where it is, and the key under which what it says is kept. */
export interface ExtraTarget {
  /** Names the file and its state (folder, name, size and time), so that a changed file is asked about again. */
  key: string;
  /** The real folder that holds the file, and its name in it. */
  folder: string;
  name: string;
  /** Looks like a picture whose size can be read: only those have dimensions to ask for. */
  picture: boolean;
}

/** What the pane wants known now, the most wanted first. */
export interface ExtraWants {
  dimensions: readonly ExtraTarget[];
  checksums: readonly ExtraTarget[];
  /** The folders that hold the items in view, whose git status is wanted (one answer covers a whole folder). */
  git: readonly string[];
  /** How many times the listing has been reloaded: a git status kept before that is out of date. */
  reloads: number;
}

/** What a pane that shows no extra column wants. */
export const NOTHING: ExtraWants = { dimensions: [], checksums: [], git: [], reloads: 0 };

/** The SHA-256 of a file, or why there is none (too big, or not readable). */
export type Checksum = { hex: string } | { problem: string };

/** Where an item is for the Git column: the real folder that holds it and its name there. */
export interface GitPlace {
  folder: string;
  name: string;
}

/** What is known of one folder's git status: the marks of its entries by name, why there are none, or `null` outside a repository. */
export type GitFolder = { marks: ReadonlyMap<string, GitMark> } | { problem: string } | null;

/** The colour of a cell's text, by the name of the variable that gives it. */
export type Tone = 'accent' | 'ok' | 'danger' | 'dim';

/** What a cell shows, and what its tooltip says (the full value). */
export interface Cell {
  text: string;
  tip?: string;
  tone?: Tone;
}

/** The marks, the most serious first (a sort by the column goes by this order), with the letter, the words and the colour of each. */
const GIT_MARKS: readonly { mark: GitMark; text: string; tip: string; tone: Tone }[] = [
  { mark: 'conflicted', text: 'U', tip: 'Conflicted: a merge left conflicts in it', tone: 'danger' },
  { mark: 'modified', text: 'M', tip: 'Modified: changed since the last commit, not staged', tone: 'accent' },
  { mark: 'staged', text: 'S', tip: 'Staged: added to the next commit', tone: 'ok' },
  { mark: 'untracked', text: '?', tip: 'Untracked: git does not know it yet', tone: 'dim' },
  { mark: 'ignored', text: '!', tip: 'Ignored: git leaves it out', tone: 'dim' },
];

/**
 * What to ask for to fill the extra columns for `entry` of `listing`, or `undefined` when it has none: a folder, `..`,
 * a special file, and anything inside an archive (reading it would unpack it). A search result or a flat view has the
 * real folder worked out from the path of the item.
 */
export function extraTargetOf(listing: Listing, entry: Entry | undefined): ExtraTarget | undefined {
  const place = placeOf(listing, entry);
  return place && { ...place, picture: hasDimensions(place.name) };
}

/**
 * What the items of `view` in the run `visible`, then those in `near` (which holds them), ask for in the columns that
 * are `shown`: the order is the order to fetch in.
 */
export function wantedExtras(view: PaneView, visible: Range, near: Range, shown: Record<ExtraColumn, boolean>): ExtraWants {
  if (!shown.dimensions && !shown.checksum && !shown.git) {
    return NOTHING;
  }

  const dimensions: ExtraTarget[] = [];
  const checksums: ExtraTarget[] = [];
  const folders = new Set<string>();

  for (const row of inFetchOrder(visible, near)) {
    const entry = view.entryAt(row);
    const target = extraTargetOf(view.listing, entry);

    // Any item has a git status, a folder too; one request answers for all the items of a folder.
    const at = shown.git ? whereIs(view.listing, entry) : undefined;
    if (at !== undefined) {
      folders.add(at.folder);
    }

    if (target === undefined) {
      continue;
    }

    if (shown.dimensions && target.picture) {
      dimensions.push(target);
    }
    if (shown.checksum) {
      checksums.push(target);
    }
  }

  return { dimensions, checksums, git: [...folders], reloads: view.listing.reloads };
}

/** The Dimensions cell: empty until the size is known (and for what is no picture), a dash for a picture with none. */
export function dimensionsCell(target: ExtraTarget | undefined, value: Dimensions | null | undefined): Cell {
  if (target === undefined || !target.picture || value === undefined) {
    return { text: '' };
  }

  if (value === null) {
    return { text: NONE, tip: 'No dimensions' };
  }

  return { text: `${value.width}×${value.height}`, tip: `${value.width} × ${value.height} pixels` };
}

/** The Checksum cell: empty until it is known, the first digits with the whole digest as tooltip, or a dash with the reason. */
export function checksumCell(target: ExtraTarget | undefined, value: Checksum | undefined): Cell {
  if (target === undefined || value === undefined) {
    return { text: '' };
  }

  if ('problem' in value) {
    return { text: NONE, tip: displayName(value.problem) };
  }

  return { text: value.hex.slice(0, CHECKSUM_DIGITS), tip: value.hex };
}

/** What a pane sorts a file by in the Dimensions column: its pixel count; `undefined` while it is not known. */
export function pixelsOf(value: Dimensions | null | undefined): number | undefined {
  return value ? value.width * value.height : undefined;
}

/** The Git cell: empty until the folder is known, for a clean item and outside a repository; a dash with the reason for a folder too big to read. */
export function gitCell(place: GitPlace | undefined, folder: GitFolder | undefined): Cell {
  if (place === undefined || !folder) {
    return { text: '' };
  }

  if ('problem' in folder) {
    return { text: NONE, tip: displayName(folder.problem) };
  }

  const found = GIT_MARKS.find(({ mark }) => mark === folder.marks.get(place.name));
  return found ? { text: found.text, tip: found.tip, tone: found.tone } : { text: '' };
}

/** What a pane sorts a file by in the Git column: how serious its mark is (0 is the most); `undefined` for an item with none, and while it is not known. */
export function severityOf(place: GitPlace | undefined, folder: GitFolder | undefined): number | undefined {
  if (place === undefined || !folder || 'problem' in folder) {
    return undefined;
  }

  const index = GIT_MARKS.findIndex(({ mark }) => mark === folder.marks.get(place.name));
  return index < 0 ? undefined : index;
}
