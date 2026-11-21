import { describe, expect, it } from 'vitest';

import { hexRow, lastTop, MAX_READ, offsetDigits, parseHex, ROW_BYTES, rowStart, windowBytes } from './hex';

const bytes = (...values: number[]) => Uint8Array.from(values);

describe('hexRow', () => {
  it('shows the offset, the bytes in hex and the printable ones as ASCII', () => {
    const row = hexRow(32, bytes(0x4d, 0x5a, 0x00, 0x7f, 0x20, 0x7e), 32, 8);

    expect(row.label).toBe('00000020');
    expect(row.cells.map((cell) => cell.hex)).toEqual(['4d', '5a', '00', '7f', '20', '7e']);
    expect(row.cells.map((cell) => cell.char).join('')).toBe('MZ.. ~');
  });

  it('is 16 bytes long, and shorter at the end of the file', () => {
    const all = Uint8Array.from({ length: 20 }, (_, index) => index);

    expect(hexRow(0, all, 0, 8).cells).toHaveLength(16);
    expect(hexRow(16, all, 0, 8).cells).toHaveLength(4);
    expect(hexRow(16, all, 0, 8).cells[0].hex).toBe('10');
  });

  it('reads from the bytes it was given, whatever offset they start at', () => {
    expect(hexRow(48, bytes(1, 2, 3), 48, 8).cells.map((cell) => cell.hex)).toEqual(['01', '02', '03']);
    expect(hexRow(64, Uint8Array.from({ length: 48 }, (_, index) => index), 48, 8).cells[0].hex).toBe('10');
  });

  it('marks the bytes of a match, also where it starts in the middle of a row', () => {
    const row = hexRow(0, Uint8Array.from({ length: 16 }, (_, index) => index), 0, 8, { offset: 14, len: 4 });

    expect(row.cells.filter((cell) => cell.hit).map((cell) => cell.hex)).toEqual(['0e', '0f']);
    expect(hexRow(16, Uint8Array.from({ length: 4 }, (_, index) => index + 16), 16, 8, { offset: 14, len: 4 }).cells.filter((cell) => cell.hit)).toHaveLength(2);
  });
});

describe('offsets', () => {
  it('rows start at multiples of 16', () => {
    expect(ROW_BYTES).toBe(16);
    expect(rowStart(0)).toBe(0);
    expect(rowStart(15)).toBe(0);
    expect(rowStart(16)).toBe(16);
    expect(rowStart(1000)).toBe(992);
    expect(rowStart(-5)).toBe(0);
  });

  it('the last page ends at the end of the file', () => {
    expect(lastTop(1000, 10)).toBe(53 * 16); // 63 rows, the last 10 start at row 53
    expect(lastTop(100, 10)).toBe(0); // 7 rows fit
    expect(lastTop(0, 10)).toBe(0);
    expect(lastTop(160, 10)).toBe(0);
    expect(lastTop(161, 10)).toBe(16);
  });

  it('offsets are written with at least 8 digits, more for a file that needs them', () => {
    expect(offsetDigits(0)).toBe(8);
    expect(offsetDigits(1000)).toBe(8);
    expect(offsetDigits(2 ** 32)).toBe(8);
    expect(offsetDigits(2 ** 36)).toBe(9);
  });

  it('a read holds three pages and never more than the backend gives', () => {
    expect(windowBytes(10)).toBe(480);
    expect(windowBytes(0)).toBe(48);
    expect(windowBytes(1_000_000)).toBe(MAX_READ);
  });
});

describe('parseHex', () => {
  it('reads pairs of digits with or without separators and 0x prefixes', () => {
    expect(parseHex('4d 5a')).toEqual([0x4d, 0x5a]);
    expect(parseHex('4D5A90')).toEqual([0x4d, 0x5a, 0x90]);
    expect(parseHex('0x4d, 0x5a')).toEqual([0x4d, 0x5a]);
    expect(parseHex('4d:5a')).toEqual([0x4d, 0x5a]);
    expect(parseHex('')).toEqual([]);
  });

  it('refuses an odd digit and anything that is not hex', () => {
    expect(parseHex('4d5')).toBeUndefined();
    expect(parseHex('zz')).toBeUndefined();
    expect(parseHex('4d 5g')).toBeUndefined();
    expect(parseHex('40x5')).toBeUndefined();
  });
});
