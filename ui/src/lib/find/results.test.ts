import { describe, expect, it } from 'vitest';

import { groupResults, joinResult, splitResult } from './results';

describe('joinResult', () => {
  it('puts the names below the folder searched, with the separator of the folder', () => {
    expect(joinResult('/home/me', 'docs/a.txt')).toBe('/home/me/docs/a.txt');
    expect(joinResult('/', 'a.txt')).toBe('/a.txt');
    expect(joinResult('C:\\Users\\me', 'docs/deep/a.txt')).toBe('C:\\Users\\me\\docs\\deep\\a.txt');
    expect(joinResult('C:\\', 'a.txt')).toBe('C:\\a.txt');
  });
});

describe('splitResult', () => {
  it('is the folder that holds the result and its name in it', () => {
    expect(splitResult('/home/me', 'docs/deep/a.txt')).toEqual({ folder: '/home/me/docs/deep', name: 'a.txt' });
    expect(splitResult('C:\\Users\\me', 'docs/a.txt')).toEqual({ folder: 'C:\\Users\\me\\docs', name: 'a.txt' });
  });

  it('is the folder searched for a result right in it', () => {
    expect(splitResult('/home/me', 'a.txt')).toEqual({ folder: '/home/me', name: 'a.txt' });
  });
});

describe('groupResults', () => {
  it('groups the results by their folder, in the order the folders first appear', () => {
    expect(groupResults('/r', ['x/a', 'b', 'x/c', 'y/z/d', 'e'])).toEqual([
      { folder: '/r/x', names: ['a', 'c'], rels: ['x/a', 'x/c'] },
      { folder: '/r', names: ['b', 'e'], rels: ['b', 'e'] },
      { folder: '/r/y/z', names: ['d'], rels: ['y/z/d'] },
    ]);
  });
});
