import type { CompareFacts } from '$lib/ipc/bindings/CompareFacts';
import type { CompareRow } from '$lib/ipc/bindings/CompareRow';
import type { CompareStatus } from '$lib/ipc/bindings/CompareStatus';
import type { SyncFacts } from '$lib/ipc/bindings/SyncFacts';
import type { SyncItem } from '$lib/ipc/bindings/SyncItem';

/** What is done with a row: copied one way or the other, or left alone. */
export type SyncAction = 'toRight' | 'toLeft' | 'skip';

/** Which of the two sides is an archive (archives are only copied from). */
export interface Archives {
  left: boolean;
  right: boolean;
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

/** What an arrow into an archive says: nothing is written into one here. */
export const ARCHIVE_TARGET_TEXT = 'Archives can only be copied from';

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

/** Why `row` cannot be copied in `direction`, or `undefined` when it can. */
export function whyNot(row: CompareRow, direction: Exclude<SyncAction, 'skip'>, archives: Archives): string | undefined {
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

  return (toRight ? archives.right : archives.left) ? ARCHIVE_TARGET_TEXT : undefined;
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
}

/** A row with a direction: the two sides as the scan saw them. */
interface Copy {
  path: string;
  direction: Exclude<SyncAction, 'skip'>;
  source: CompareFacts;
  target: CompareFacts | null;
}

/** The rows that are copied, in the order of the scan. */
function copies(rows: readonly SyncRow[]): Copy[] {
  const found: Copy[] = [];

  for (const { row, action } of rows) {
    const [source, target] = action === 'toRight' ? [row.left, row.right] : [row.right, row.left];
    if (action !== 'skip' && source !== null) {
      found.push({ path: row.path, direction: action, source, target });
    }
  }

  return found;
}

/** What the copies add up to; a folder only one side has counts its whole subtree. */
export function totalsOf(rows: readonly SyncRow[]): Totals {
  const totals: Totals = { toRight: { items: 0, bytes: 0 }, toLeft: { items: 0, bytes: 0 }, overwrites: 0 };

  for (const { direction, source, target } of copies(rows)) {
    totals[direction].items += 1;
    totals[direction].bytes += source.kind === 'dir' ? source.bytes : source.size;
    totals.overwrites += target === null ? 0 : 1;
  }

  return totals;
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
