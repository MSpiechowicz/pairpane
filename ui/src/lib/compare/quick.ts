import type { Entry } from '$lib/ipc/bindings/Entry';

/** How an entry of a pane stands against the entry of the same name in the other pane. */
export type Mark = 'onlyHere' | 'newer' | 'older' | 'differs';

/** Times this close (in milliseconds) are the same time: filesystems keep them to different precision. */
export const TOLERANCE_MS = 2000;

/** What each mark says; a mark is told in words as well as in a tint (a title and an accessible description). */
export const MARK_TEXT: Record<Mark, string> = {
  onlyHere: 'Only in this pane',
  newer: 'Newer than in the other pane',
  older: 'Older than in the other pane',
  differs: 'Differs from the other pane',
};

/** How many entries of each pane stand each way. */
export interface QuickCounts {
  leftOnly: number;
  rightOnly: number;
  leftNewer: number;
  rightNewer: number;
  differs: number;
}

export interface QuickResult {
  left: Map<string, Mark>;
  right: Map<string, Mark>;
  /** The names to select in each pane: the entries that are only there or are newer there (what a copy to the other side takes). */
  select: { left: string[]; right: string[] };
  counts: QuickCounts;
}

type Standing = Mark | undefined;

/** The entries by name, the first of names that fold together (the others are not compared). */
function byName(entries: readonly (Entry | undefined)[], foldCase: boolean): Map<string, Entry> {
  const named = new Map<string, Entry>();
  for (const entry of entries) {
    if (entry === undefined) {
      continue;
    }

    const key = foldCase ? entry.name.toLowerCase() : entry.name;
    if (!named.has(key)) {
      named.set(key, entry);
    }
  }

  return named;
}

/**
 * How `left` stands against `right`, as the marks of the left entry and of the right one. Folders are the same by
 * being there; a file and a folder differ; files are the same when their times are within {@link TOLERANCE_MS} and
 * their sizes are equal, and otherwise the later one is newer (whatever the sizes).
 */
function standing(left: Entry, right: Entry): [Standing, Standing] {
  if (left.kind !== right.kind) {
    return ['differs', 'differs'];
  }

  if (left.kind === 'dir') {
    return [undefined, undefined];
  }

  if (left.modified !== null && right.modified !== null) {
    const later = left.modified - right.modified;
    if (later > TOLERANCE_MS) {
      return ['newer', 'older'];
    }

    if (later < -TOLERANCE_MS) {
      return ['older', 'newer'];
    }
  }

  return left.size === right.size ? [undefined, undefined] : ['differs', 'differs'];
}

/**
 * Compares two listings by name (folded when the filesystem does not tell cases apart), size and modification time.
 * Entries not yet delivered are left out.
 */
export function compareListings(left: readonly (Entry | undefined)[], right: readonly (Entry | undefined)[], foldCase: boolean): QuickResult {
  const result: QuickResult = {
    left: new Map(),
    right: new Map(),
    select: { left: [], right: [] },
    counts: { leftOnly: 0, rightOnly: 0, leftNewer: 0, rightNewer: 0, differs: 0 },
  };
  const here = byName(left, foldCase);
  const there = byName(right, foldCase);

  for (const [key, entry] of here) {
    const other = there.get(key);
    if (other === undefined) {
      result.left.set(entry.name, 'onlyHere');
    } else {
      const [mine, theirs] = standing(entry, other);
      if (mine !== undefined) {
        result.left.set(entry.name, mine);
      }

      if (theirs !== undefined) {
        result.right.set(other.name, theirs);
      }
    }
  }

  for (const [key, entry] of there) {
    if (!here.has(key)) {
      result.right.set(entry.name, 'onlyHere');
    }
  }

  tally(result);
  return result;
}

/** Counts the marks and picks the names to select. */
function tally({ left, right, select, counts }: QuickResult) {
  for (const [name, mark] of left) {
    counts.leftOnly += mark === 'onlyHere' ? 1 : 0;
    counts.leftNewer += mark === 'newer' ? 1 : 0;
    counts.differs += mark === 'differs' ? 1 : 0;
    if (mark === 'onlyHere' || mark === 'newer') {
      select.left.push(name);
    }
  }

  for (const [name, mark] of right) {
    counts.rightOnly += mark === 'onlyHere' ? 1 : 0;
    counts.rightNewer += mark === 'newer' ? 1 : 0;
    if (mark === 'onlyHere' || mark === 'newer') {
      select.right.push(name);
    }
  }
}

const PARTS: readonly { key: keyof QuickCounts; text: string }[] = [
  { key: 'leftOnly', text: 'only on the left' },
  { key: 'rightOnly', text: 'only on the right' },
  { key: 'leftNewer', text: 'newer on the left' },
  { key: 'rightNewer', text: 'newer on the right' },
  { key: 'differs', text: 'differ' },
];

/** What the card says: `12 only on the left, 3 newer on the right.` */
export function quickSummary(counts: QuickCounts): string {
  const parts = PARTS.filter(({ key }) => counts[key] > 0).map(({ key, text }) => `${counts[key].toLocaleString('en-US')} ${text}`);
  return parts.length === 0 ? 'The folders match.' : `${parts.join(', ')}.`;
}
