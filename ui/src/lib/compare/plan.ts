import type { CompareFacts } from '$lib/ipc/bindings/CompareFacts';
import type { CompareRow } from '$lib/ipc/bindings/CompareRow';
import type { CompareStatus } from '$lib/ipc/bindings/CompareStatus';
import type { SyncFacts } from '$lib/ipc/bindings/SyncFacts';
import type { SyncItem } from '$lib/ipc/bindings/SyncItem';
import type { SyncRemoval } from '$lib/ipc/bindings/SyncRemoval';

/** What is done with a row: copied one way or the other, removed from the one side that has it (a mirror), or left alone. */
export type SyncAction = 'toRight' | 'toLeft' | 'remove' | 'skip';

/** The directions a row can be copied in. */
export type SyncCopy = 'toRight' | 'toLeft';

/** Which of the two sides is an archive, and which of those can only be read (a RAR, a lone `.gz`): it is only copied from. */
export interface Archives {
  left: boolean;
  right: boolean;
  leftReadOnly: boolean;
  rightReadOnly: boolean;
}

/** A row of the table: what the scan found, and what is to be done about it. */
export interface SyncRow {
  readonly row: CompareRow;
  action: SyncAction;
  /** Chosen in the table, for the bulk change of actions. */
  picked: boolean;
}

export const STATUS_TEXT: Record<CompareStatus, string> = {
  leftOnly: 'Only left',
  rightOnly: 'Only right',
  leftNewer: 'Left newer',
  rightNewer: 'Right newer',
  differs: 'Differs',
  equal: 'Equal',
};

/** What an arrow into, or a removal from, an archive that is only read says. */
export const ARCHIVE_TARGET_TEXT = 'This archive can only be read (a RAR or a single .gz file)';

/** The direction each status is copied in unless the user chooses otherwise; a difference that says no way (`differs`) is left alone. */
const DEFAULTS: Record<CompareStatus, SyncAction> = {
  leftOnly: 'toRight',
  rightOnly: 'toLeft',
  leftNewer: 'toRight',
  rightNewer: 'toLeft',
  differs: 'skip',
  equal: 'skip',
};

/** The name of the item: the last part of its path. */
export function baseName(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1);
}

/** Why `row` cannot be removed, or `undefined` when it can: only an item that one side alone has, a file or a folder. */
function whyNotRemove(row: CompareRow, archives: Archives): string | undefined {
  const { left, right } = row;
  const side = left ?? right;

  if (side === null || (left !== null && right !== null)) {
    return 'Only an item that one side alone has is removed.';
  }

  if (side.link || side.kind === 'other') {
    return 'Links and special files are not removed.';
  }

  return (left !== null ? archives.leftReadOnly : archives.rightReadOnly) ? ARCHIVE_TARGET_TEXT : undefined;
}

/** Why `row` cannot be copied in `direction` (or removed), or `undefined` when it can. */
export function whyNot(row: CompareRow, direction: Exclude<SyncAction, 'skip'>, archives: Archives): string | undefined {
  if (direction === 'remove') {
    return whyNotRemove(row, archives);
  }

  const { left, right } = row;
  const toRight = direction === 'toRight';

  if (row.status === 'equal') {
    return 'The same on both sides.';
  }

  if (left?.link || right?.link) {
    return 'Links are not copied.';
  }

  if (left?.kind === 'other' || right?.kind === 'other') {
    return 'Special files are not copied.';
  }

  if (left && right && left.kind !== right.kind) {
    return 'A file and a folder cannot replace each other.';
  }

  if ((toRight ? left : right) === null) {
    return 'There is nothing on this side to copy.';
  }

  return (toRight ? archives.rightReadOnly : archives.leftReadOnly) ? ARCHIVE_TARGET_TEXT : undefined;
}

/**
 * What a mirror `toward` one side does with a row, so that side matches the other: what the source has is copied over (a
 * newer target too), what only the target has is removed, and what is equal is left alone. `undefined` when the row cannot
 * go that way.
 */
export function mirrorAction(row: CompareRow, toward: SyncCopy, archives: Archives): SyncAction | undefined {
  const targetOnly = toward === 'toRight' ? 'rightOnly' : 'leftOnly';
  const action: SyncAction = row.status === 'equal' ? 'skip' : row.status === targetOnly ? 'remove' : toward;

  return action === 'skip' || whyNot(row, action, archives) === undefined ? action : undefined;
}

/** What a row starts with: the direction its status asks for, when it can go that way. */
export function defaultAction(row: CompareRow, archives: Archives): SyncAction {
  const action = DEFAULTS[row.status];
  return action !== 'skip' && whyNot(row, action, archives) === undefined ? action : 'skip';
}

/** What the copies in one direction add up to. */
export interface Total {
  items: number;
  bytes: number;
}

export interface Totals {
  toRight: Total;
  toLeft: Total;
  /** Items that replace a file that is there. */
  overwrites: number;
  /** What is removed from the left side and from the right one. */
  removeLeft: Total;
  removeRight: Total;
}

/** A row with a direction: the two sides as the scan saw them. */
interface Copy {
  path: string;
  direction: SyncCopy;
  source: CompareFacts;
  target: CompareFacts | null;
}

/** A row that is removed: the side that has it, as the scan saw it. */
interface Removal {
  path: string;
  side: 'left' | 'right';
  seen: CompareFacts;
}

/** The rows that are copied, in the order of the scan. */
function copies(rows: readonly SyncRow[]): Copy[] {
  const found: Copy[] = [];

  for (const { row, action } of rows) {
    const [source, target] = action === 'toRight' ? [row.left, row.right] : [row.right, row.left];
    if ((action === 'toRight' || action === 'toLeft') && source !== null) {
      found.push({ path: row.path, direction: action, source, target });
    }
  }

  return found;
}

/** The rows that are removed, in the order of the scan. */
function removals(rows: readonly SyncRow[]): Removal[] {
  const found: Removal[] = [];

  for (const { row, action } of rows) {
    const seen = row.left ?? row.right;
    if (action === 'remove' && seen !== null) {
      found.push({ path: row.path, side: row.left !== null ? 'left' : 'right', seen });
    }
  }

  return found;
}

/** The bytes of an item: a folder only one side has counts its whole subtree. */
const bytesOf = (facts: CompareFacts) => (facts.kind === 'dir' ? facts.bytes : facts.size);

/** What the copies and the removals add up to. */
export function totalsOf(rows: readonly SyncRow[]): Totals {
  const none = () => ({ items: 0, bytes: 0 });
  const totals: Totals = { toRight: none(), toLeft: none(), overwrites: 0, removeLeft: none(), removeRight: none() };

  for (const { direction, source, target } of copies(rows)) {
    totals[direction].items += 1;
    totals[direction].bytes += bytesOf(source);
    totals.overwrites += target === null ? 0 : 1;
  }

  for (const { side, seen } of removals(rows)) {
    const total = side === 'left' ? totals.removeLeft : totals.removeRight;
    total.items += 1;
    total.bytes += bytesOf(seen);
  }

  return totals;
}

/** How many items the totals copy or remove. */
export function itemCount({ toRight, toLeft, removeLeft, removeRight }: Totals): number {
  return toRight.items + toLeft.items + removeLeft.items + removeRight.items;
}

const factsOf = ({ kind, size, modified }: CompareFacts): SyncFacts => ({ kind, size, modified });

/** The copies the job is asked for: each row with a direction, with the two sides as the scan saw them. */
export function itemsOf(rows: readonly SyncRow[]): SyncItem[] {
  return copies(rows).map(({ path, direction, source, target }) => ({
    path,
    direction,
    source: factsOf(source),
    target: target === null ? null : factsOf(target),
  }));
}

/**
 * The removals the job is asked for: each removed row, as the scan saw it (for a folder, with its subtree's totals), and
 * when the scan began (`scanned`, milliseconds since the epoch): a folder with anything newer in it is kept.
 */
export function removalsOf(rows: readonly SyncRow[], scanned: number): SyncRemoval[] {
  return removals(rows).map(({ path, side, seen }) => ({
    path,
    side,
    seen: factsOf(seen),
    files: seen.kind === 'dir' ? seen.files : 0,
    bytes: seen.kind === 'dir' ? seen.bytes : 0,
    scanned,
  }));
}

/** The paths that are removed from each side, in the order of the scan. */
export function removedPaths(rows: readonly SyncRow[]): { side: 'left' | 'right'; path: string }[] {
  return removals(rows).map(({ path, side }) => ({ path, side }));
}
