import { describe, expect, it } from 'vitest';

import { matchLines, type LineBlock } from './lineMatch';

/** Ids for lines written as letters: equal letters, equal lines. */
const ids = (text: string) => Array.from(text, (letter) => letter.charCodeAt(0) - 97);
const match = (a: string, b: string, idCount = 26) => matchLines(ids(a), ids(b), idCount);

/** The same letters read back from the blocks of a diff: what A keeps must be what B keeps, in order, and the blocks must not touch or overlap. */
function expectConsistent(a: number[], b: number[], blocks: LineBlock[]) {
  let posA = 0;
  let posB = 0;

  for (const block of blocks) {
    expect(block.aFrom).toBeGreaterThanOrEqual(posA);
    expect(block.bFrom).toBeGreaterThanOrEqual(posB);
    expect(a.slice(posA, block.aFrom)).toEqual(b.slice(posB, block.bFrom));
    expect(block.aTo - block.aFrom + block.bTo - block.bFrom).toBeGreaterThan(0);
    posA = block.aTo;
    posB = block.bTo;
  }

  expect(a.slice(posA)).toEqual(b.slice(posB));
}

describe('matching lines', () => {
  it('finds nothing in equal sequences, and one block in sequences with nothing shared', () => {
    expect(match('abc', 'abc')).toEqual([]);
    expect(match('', '')).toEqual([]);
    expect(match('abc', 'xyz')).toEqual([{ aFrom: 0, aTo: 3, bFrom: 0, bTo: 3 }]);
  });

  it('finds an insertion and a deletion at the start, the middle and the end', () => {
    expect(match('abc', 'xabc')).toEqual([{ aFrom: 0, aTo: 0, bFrom: 0, bTo: 1 }]);
    expect(match('abc', 'abxc')).toEqual([{ aFrom: 2, aTo: 2, bFrom: 2, bTo: 3 }]);
    expect(match('abc', 'abcx')).toEqual([{ aFrom: 3, aTo: 3, bFrom: 3, bTo: 4 }]);
    expect(match('xabc', 'abc')).toEqual([{ aFrom: 0, aTo: 1, bFrom: 0, bTo: 0 }]);
    expect(match('abxc', 'abc')).toEqual([{ aFrom: 2, aTo: 3, bFrom: 2, bTo: 2 }]);
    expect(match('abcx', 'abc')).toEqual([{ aFrom: 3, aTo: 4, bFrom: 3, bTo: 3 }]);
  });

  it('finds a replaced line as one block, and replaced lines apart as two', () => {
    expect(match('abc', 'aXc')).toEqual([{ aFrom: 1, aTo: 2, bFrom: 1, bTo: 2 }]);
    expect(match('abcde', 'aXcYe')).toHaveLength(2);
  });

  it('handles the empty sides', () => {
    expect(match('', 'abc')).toEqual([{ aFrom: 0, aTo: 0, bFrom: 0, bTo: 3 }]);
    expect(match('abc', '')).toEqual([{ aFrom: 0, aTo: 3, bFrom: 0, bTo: 0 }]);
  });

  it('shows a moved block as a deletion and an insertion (or a replacement) round the lines that stay', () => {
    const a = ids('abcdefghij');
    const b = ids('abfghicdej');
    const blocks = matchLines(a, b, 26);

    expectConsistent(a, b, blocks);
    expect(blocks.length).toBeGreaterThanOrEqual(2);
    expect(blocks.length).toBeLessThanOrEqual(3);
  });

  it('keeps unique lines as the fixed points between repeated ones', () => {
    // The repeated "x" lines on either side of a unique line need no search of their own.
    expect(match('xxxqxxx', 'xxxqyxxx')).toEqual([{ aFrom: 4, aTo: 4, bFrom: 4, bTo: 5 }]);
  });

  it('is consistent for random sequences, and no longer than deleting and adding everything', () => {
    let seed = 99;
    const next = (limit: number) => (seed = (seed * 1103515245 + 12345) >>> 0) % limit;

    for (let round = 0; round < 600; round++) {
      const a = Array.from({ length: next(14) }, () => next(4));
      const b = Array.from({ length: next(14) }, () => next(4));
      const blocks = matchLines(a, b, 4);
      const edits = blocks.reduce((sum, block) => sum + block.aTo - block.aFrom + block.bTo - block.bFrom, 0);

      expectConsistent(a, b, blocks);
      expect(edits).toBeLessThanOrEqual(a.length + b.length);
    }
  });

  it('finds the shortest edit script of a stretch with no unique line', () => {
    const blocks = match('ababababab', 'bababababa');

    expect(blocks.reduce((sum, block) => sum + block.aTo - block.aFrom + block.bTo - block.bFrom, 0)).toBe(2);
  });

  it('gives up a stretch of repeated lines that needs too many edits as one replaced block (right, only coarser)', () => {
    // No line is unique, and 1500 edits are more than the search takes on.
    const a = Array.from({ length: 1500 }, (_, index) => index % 2);
    const b = Array.from({ length: 1500 }, (_, index) => (index + 1) % 2);
    const blocks = matchLines(a, b, 2);

    expectConsistent(a, b, blocks);
    expect(blocks.length).toBeLessThan(5);
  });

  it('finishes a large repetitive pair at once, whatever the edits', () => {
    const size = 400_000;
    const a = Array.from({ length: size }, () => 0);
    const b = Array.from({ length: size }, (_, index) => (index % 97 === 0 ? 1 : 0));
    const started = performance.now();
    const blocks = matchLines(a, b, 2);

    expect(performance.now() - started).toBeLessThan(2000);
    expectConsistent(a, b, blocks);
  });
});
