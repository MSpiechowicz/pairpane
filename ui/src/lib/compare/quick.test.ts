import { describe, expect, it } from 'vitest';

import { entry } from '../../../tests/panes';
import { compareListings, quickSummary, TOLERANCE_MS } from './quick';

const T = Date.UTC(2026, 10, 20, 12, 0);
const file = (name: string, modified: number | null, size = 10) => entry(name, 'file', { modified, size });

/** The marks of the two panes for one file each, `left` against `right`. */
const against = (left: ReturnType<typeof file>, right: ReturnType<typeof file>) => {
  const { left: here, right: there } = compareListings([left], [right], false);
  return [here.get(left.name), there.get(right.name)];
};

describe('compareListings', () => {
  it('marks what only one side has', () => {
    const result = compareListings([entry('a.txt'), entry('both')], [entry('both'), entry('b.txt')], false);

    expect([...result.left]).toEqual([['a.txt', 'onlyHere']]);
    expect([...result.right]).toEqual([['b.txt', 'onlyHere']]);
    expect(result.select).toEqual({ left: ['a.txt'], right: ['b.txt'] });
  });

  it('times within two seconds are one time; the sizes then decide', () => {
    expect(against(file('f', T + TOLERANCE_MS - 1), file('f', T))).toEqual([undefined, undefined]);
    expect(against(file('f', T + TOLERANCE_MS), file('f', T))).toEqual([undefined, undefined]);
    expect(against(file('f', T - TOLERANCE_MS), file('f', T))).toEqual([undefined, undefined]);
  });

  it('a time more than two seconds later is newer, whatever the sizes', () => {
    expect(against(file('f', T + TOLERANCE_MS + 1), file('f', T))).toEqual(['newer', 'older']);
    expect(against(file('f', T), file('f', T + TOLERANCE_MS + 1))).toEqual(['older', 'newer']);
    expect(against(file('f', T + 60_000, 1), file('f', T, 999))).toEqual(['newer', 'older']);
  });

  it('the same time and different sizes differ, and so do sizes when a time is unknown', () => {
    expect(against(file('f', T, 10), file('f', T + 1000, 11))).toEqual(['differs', 'differs']);
    expect(against(file('f', null, 10), file('f', T, 11))).toEqual(['differs', 'differs']);
    expect(against(file('f', null, 10), file('f', null, 10))).toEqual([undefined, undefined]);
  });

  it('folders are compared by being there; a folder and a file of one name differ', () => {
    const folders = compareListings([entry('d', 'dir', { modified: T })], [entry('d', 'dir', { modified: T + 99_000 })], false);
    expect(folders.left.size + folders.right.size).toBe(0);

    const mixed = compareListings([entry('x', 'dir')], [file('x', T)], false);
    expect([...mixed.left, ...mixed.right]).toEqual([
      ['x', 'differs'],
      ['x', 'differs'],
    ]);
  });

  it('selects on each side what is only there or newer there, not what differs or is older', () => {
    const result = compareListings(
      [file('only', T), file('new', T + 60_000), file('old', T), file('diff', T, 1)],
      [file('new', T), file('old', T + 60_000), file('diff', T, 2)],
      false,
    );

    expect(result.select.left).toEqual(['only', 'new']);
    expect(result.select.right).toEqual(['old']);
    expect(result.counts).toEqual({ leftOnly: 1, rightOnly: 0, leftNewer: 1, rightNewer: 1, differs: 1 });
  });

  it('pairs names that differ only in case when the filesystem folds them, and not otherwise', () => {
    const left = [file('README.md', T, 1)];
    const right = [file('readme.MD', T, 2)];

    const folded = compareListings(left, right, true);
    expect([...folded.left, ...folded.right]).toEqual([
      ['README.md', 'differs'],
      ['readme.MD', 'differs'],
    ]);

    const exact = compareListings(left, right, false);
    expect([...exact.left, ...exact.right]).toEqual([
      ['README.md', 'onlyHere'],
      ['readme.MD', 'onlyHere'],
    ]);
  });

  it('of two names that fold together on one side only the first is compared', () => {
    const result = compareListings([file('a.txt', T), file('A.TXT', T)], [file('a.txt', T)], true);

    expect(result.left.size + result.right.size).toBe(0);
  });

  it('leaves out entries whose batch has not arrived', () => {
    const result = compareListings([undefined, entry('a')], [undefined], false);

    expect([...result.left]).toEqual([['a', 'onlyHere']]);
  });
});

describe('quickSummary', () => {
  it('counts what is there, in the order of the sides', () => {
    expect(quickSummary({ leftOnly: 12, rightOnly: 0, leftNewer: 0, rightNewer: 3, differs: 0 })).toBe('12 only on the left, 3 newer on the right.');
    expect(quickSummary({ leftOnly: 1, rightOnly: 2, leftNewer: 3, rightNewer: 4, differs: 5 })).toBe(
      '1 only on the left, 2 only on the right, 3 newer on the left, 4 newer on the right, 5 differ.',
    );
  });

  it('says so when the folders match', () => {
    expect(quickSummary({ leftOnly: 0, rightOnly: 0, leftNewer: 0, rightNewer: 0, differs: 0 })).toBe('The folders match.');
  });
});
