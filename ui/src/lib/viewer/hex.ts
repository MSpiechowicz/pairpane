/** The hex view's arithmetic and text. A row is 16 bytes: its offset, the bytes in hex and their ASCII. */

import type { SearchHit } from '$lib/ipc/bindings/SearchHit';

/** Bytes in a row. */
export const ROW_BYTES = 16;

/** The most bytes one `view_bytes` returns (the backend's cap). */
export const MAX_READ = 4 * 1024 * 1024;

/** How many rows to read at once: the viewport's, and a page more on each side, within what one read may hold. */
export function windowBytes(rows: number): number {
  return Math.min(MAX_READ, Math.max(1, rows) * ROW_BYTES * 3);
}

/** The start of the row `offset` falls in. */
export function rowStart(offset: number): number {
  return Math.max(0, Math.floor(offset / ROW_BYTES) * ROW_BYTES);
}

/** The first offset of the last page: the furthest the view scrolls. */
export function lastTop(size: number, rows: number): number {
  const total = Math.ceil(size / ROW_BYTES);

  return Math.max(0, total - Math.max(1, rows)) * ROW_BYTES;
}

/** How many hex digits the offsets of a file of `size` bytes are written with (at least 8). */
export function offsetDigits(size: number): number {
  return Math.max(8, Math.max(0, size - 1).toString(16).length);
}

export interface HexRow {
  /** The offset of the first byte, as a number and written out. */
  offset: number;
  label: string;
  /** One entry per byte of the row (fewer in the last one). */
  cells: { hex: string; char: string; hit: boolean }[];
}

/** The printable ASCII character of a byte, `.` for the rest. */
const printable = (byte: number) => (byte >= 0x20 && byte < 0x7f ? String.fromCharCode(byte) : '.');

/** The row that starts at `offset`, from the bytes read from `base`; the bytes of `hit` are marked. */
export function hexRow(offset: number, bytes: Uint8Array, base: number, digits: number, hit?: SearchHit): HexRow {
  const cells: HexRow['cells'] = [];
  const from = offset - base;

  for (let index = 0; index < ROW_BYTES && from + index < bytes.length; index += 1) {
    const byte = bytes[from + index];
    const at = offset + index;
    cells.push({
      hex: byte.toString(16).padStart(2, '0'),
      char: printable(byte),
      hit: hit !== undefined && at >= hit.offset && at < hit.offset + hit.len,
    });
  }

  return { offset, label: offset.toString(16).padStart(digits, '0'), cells };
}

/**
 * The bytes of a hex needle: pairs of digits, with or without spaces (`4d 5a`, `4D5A`, `0x4d 0x5a`). `undefined`
 * when it holds anything else or an odd digit; an empty text is an empty needle.
 */
export function parseHex(text: string): number[] | undefined {
  const digits = text.replace(/(^|[\s,:])0x/gi, '$1').replace(/[\s,:]+/g, '');
  if (digits.length % 2 !== 0 || !/^[0-9a-f]*$/i.test(digits)) {
    return undefined;
  }

  const bytes: number[] = [];
  for (let at = 0; at < digits.length; at += 2) {
    bytes.push(Number.parseInt(digits.slice(at, at + 2), 16));
  }

  return bytes;
}
