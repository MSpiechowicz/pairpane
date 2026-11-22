import { afterEach, describe, expect, it, vi } from 'vitest';

import { groupResults, joinResult, splitResult, withNested, withoutNested } from './results';

afterEach(() => vi.restoreAllMocks());

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
      { folder: '/r/x', dir: 'x', names: ['a', 'c'], rels: ['x/a', 'x/c'] },
      { folder: '/r', dir: '', names: ['b', 'e'], rels: ['b', 'e'] },
      { folder: '/r/y/z', dir: 'y/z', names: ['d'], rels: ['y/z/d'] },
    ]);
  });

  it('leaves out a result inside another one, so that a folder and what it holds are one request', () => {
    expect(groupResults('/r', ['d', 'd/f', 'd/sub/g', 'e/h'])).toEqual([
      { folder: '/r', dir: '', names: ['d'], rels: ['d'] },
      { folder: '/r/e', dir: 'e', names: ['h'], rels: ['e/h'] },
    ]);
  });
});

describe('withoutNested', () => {
  it('drops each result that has a chosen folder above it, whatever the depth, and keeps the order', () => {
    expect(withoutNested(['a/b/c', 'a', 'x/y', 'a/b', 'x/z'])).toEqual(['a', 'x/y', 'x/z']);
  });

  it('keeps a name that only starts like a folder’s, and results that share a folder nobody chose', () => {
    expect(withoutNested(['d', 'd2/f', 'dd', 'e/f', 'e/g'])).toEqual(['d', 'd2/f', 'dd', 'e/f', 'e/g']);
  });

  it('tells Docs from docs on Linux', () => {
    expect(withoutNested(['Docs', 'docs/f'])).toEqual(['Docs', 'docs/f']);
  });

  it.each(['Mozilla/5.0 (Windows NT 10.0; Win64; x64)', 'Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)'])(
    'does not on Windows and macOS (%s)',
    (agent) => {
      vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue(agent);

      expect(withoutNested(['Docs', 'docs/f', 'DOCS/g/h'])).toEqual(['Docs']);
    },
  );
});

describe('withNested', () => {
  it('adds the selected results inside the names to them, and none else', () => {
    expect(withNested(['d', 'x/y'], ['d', 'd/f', 'x/y/z', 'x/other', 'e'])).toEqual(['d', 'x/y', 'd/f', 'x/y/z']);
  });
});
