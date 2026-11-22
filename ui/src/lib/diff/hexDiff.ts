import { hexRow, type HexRow, ROW_BYTES } from '$lib/viewer/hex';

/** One byte of the two columns: its hex and character, whether the other file has another byte there (or none), and whether the file ends before it. */
export interface DiffCell {
  hex: string;
  char: string;
  differs: boolean;
  /** The file has no byte here (it is shorter); the cell is drawn empty. */
  blank: boolean;
  /** The difference that Previous/Next last went to. */
  current: boolean;
}

/** A row of the two columns: the same 16 offsets of each file. */
export interface DiffRow {
  offset: number;
  label: string;
  left: DiffCell[];
  right: DiffCell[];
}

/** The bytes held of one file, and the offset of the first one. */
export interface Held {
  bytes: Uint8Array;
  base: number;
}

const EMPTY: DiffCell = { hex: '', char: '', differs: false, blank: true, current: false };

/** The cells of one column against the other one's. */
function column(own: HexRow['cells'], other: HexRow['cells'], offset: number, current: number | undefined): DiffCell[] {
  return Array.from({ length: ROW_BYTES }, (_, index) => {
    const cell = own[index];

    if (cell === undefined) {
      return EMPTY;
    }

    return { hex: cell.hex, char: cell.char, differs: other[index]?.hex !== cell.hex, blank: false, current: offset + index === current };
  });
}

/**
 * The row that starts at `offset`, from the bytes both files hold. A byte one file has and the other (shorter) has
 * not counts as a difference. `current` is the offset of the difference the user went to last.
 */
export function diffRow(offset: number, left: Held, right: Held, digits: number, current?: number): DiffRow {
  const own = hexRow(offset, left.bytes, left.base, digits);
  const other = hexRow(offset, right.bytes, right.base, digits);

  return {
    offset,
    label: own.label,
    left: column(own.cells, other.cells, offset, current),
    right: column(other.cells, own.cells, offset, current),
  };
}

/** `12 345`: the digits in groups of three. */
export function groupDigits(value: number): string {
  const digits = String(value);
  let grouped = '';

  for (let at = 0; at < digits.length; at += 1) {
    grouped += (at > 0 && (digits.length - at) % 3 === 0 ? ' ' : '') + digits[at];
  }

  return grouped;
}

const bytes = (count: number) => `${groupDigits(count)} ${count === 1 ? 'byte' : 'bytes'}`;

/** `0x1A2B` */
export const hexOffset = (offset: number) => `0x${offset.toString(16).toUpperCase()}`;

/**
 * What the line above the columns says: the two sizes and where the files first differ, or that they do not.
 * `first` is `undefined` while that is being looked for.
 */
export function sizeLine(left: number, right: number, first: number | 'none' | undefined): string {
  if (first === 'none') {
    return 'The files are identical.';
  }

  const sizes = `${bytes(left)} / ${bytes(right)}`;

  return first === undefined ? `${sizes}, looking for the first difference…` : `${sizes}, first difference at ${hexOffset(first)}`;
}
