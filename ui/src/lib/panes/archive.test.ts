import { afterEach, describe, expect, it, vi } from 'vitest';

import { above, archiveFile, crumbKey,
  isArchiveName,
  isWritableArchive,
  joinInner,
  joinPath,
  sameArchivePath,
} from './archive';
import { entry } from '../../../tests/panes';

describe('isArchiveName', () => {
  it.each([
    'a.zip',
    'lib.JAR',
    'a.tar',
    'a.tar.gz',
    'a.TGZ',
    'a.tar.xz',
    'a.txz',
    'a.tar.zst',
    'a.tzst',
    'a.tar.bz2',
    'a.tbz2',
    'a.tbz',
    'notes.gz',
    'a.7z',
    'a.Rar',
  ])('knows %s', (name) => expect(isArchiveName(name)).toBe(true));

  it.each(['a.txt', 'zip', '.zip', 'a.zip.bak', 'a.xz', 'a.zst', 'a.bz2'])('does not know %s', (name) =>
    expect(isArchiveName(name)).toBe(false),
  );
});

describe('isWritableArchive', () => {
  it.each(['a.zip', 'lib.JAR', 'a.tar', 'a.tar.gz', 'a.TGZ', 'a.tar.xz', 'a.txz', 'a.tar.zst', 'a.tzst', 'a.tar.bz2', 'a.tbz2', 'a.tbz', 'a.7z'])(
    'can be added to: %s',
    (name) => expect(isWritableArchive(name)).toBe(true),
  );

  it.each(['a.rar', 'a.RAR', 'notes.gz', 'a.txt', '.zip', 'a.xz'])('cannot be added to: %s', (name) =>
    expect(isWritableArchive(name)).toBe(false),
  );
});

describe('archive locations', () => {
  it('joins a file to its folder with the separator the folder uses', () => {
    expect(joinPath('/home/me', 'a.zip')).toBe('/home/me/a.zip');
    expect(joinPath('/', 'a.zip')).toBe('/a.zip');
    expect(joinPath('C:\\Users\\me', 'a.zip')).toBe('C:\\Users\\me\\a.zip');
    expect(joinPath('C:\\', 'a.zip')).toBe('C:\\a.zip');
  });

  it('joins inner folders with a slash', () => {
    expect(joinInner('', 'docs')).toBe('docs');
    expect(joinInner('docs', 'img')).toBe('docs/img');
  });

  it('goes up inside an archive and from its root to the folder that holds it', () => {
    const archive = '/work/a.zip';

    expect(above({ archive, dir: 'docs/img' }, '/work')).toEqual({ spot: { archive, dir: 'docs' }, left: 'img' });
    expect(above({ archive, dir: 'docs' }, '/work')).toEqual({ spot: { archive, dir: '' }, left: 'docs' });
    expect(above({ archive, dir: '' }, '/work')).toEqual({ folder: '/work', left: 'a.zip' });
    expect(above({ archive: 'C:\\w\\a.zip', dir: '' }, 'C:\\w')).toEqual({ folder: 'C:\\w', left: 'a.zip' });
  });

  it('opens only files named like archives', () => {
    expect(archiveFile('/work', entry('a.zip'))).toBe('/work/a.zip');
    expect(archiveFile('/work', entry('a.zip', 'dir'))).toBeUndefined();
    expect(archiveFile('/work', entry('a.txt'))).toBeUndefined();
    expect(archiveFile('/work', undefined)).toBeUndefined();
  });

  it('tells path bar steps apart by path and by folder inside the archive', () => {
    const keys = [
      { path: '/work' },
      { path: '/work/a.zip', inside: '' },
      { path: '/work/a.zip', inside: 'docs' },
      { path: '/work/a.zip', inside: 'docs/img' },
    ].map(crumbKey);

    expect(new Set(keys).size).toBe(4);
  });
});

describe('sameArchivePath', () => {
  const as = (userAgent: string) => vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue(userAgent);

  afterEach(() => vi.restoreAllMocks());

  it('matches Windows paths that differ in separator, case and a trailing separator', () => {
    as('Mozilla/5.0 (Windows NT 10.0; Win64; x64)');

    expect(sameArchivePath('c:/Users/Me/a.zip', 'C:\\users\\me\\A.ZIP')).toBe(true);
    expect(sameArchivePath('C:\\Users\\Me\\a.zip\\', 'C:\\Users\\Me\\a.zip')).toBe(true);
    expect(sameArchivePath('C:/', 'c:\\')).toBe(true);
  });

  it('tells Windows archives apart', () => {
    as('Mozilla/5.0 (Windows NT 10.0; Win64; x64)');

    expect(sameArchivePath('C:\\a\\x.zip', 'C:\\a\\y.zip')).toBe(false);
    expect(sameArchivePath('C:\\a\\x.zip', 'D:\\a\\x.zip')).toBe(false);
  });

  it('matches POSIX paths exactly, but for a trailing slash', () => {
    as('Mozilla/5.0 (X11; Linux x86_64)');

    expect(sameArchivePath('/home/me/a.zip', '/home/me/a.zip')).toBe(true);
    expect(sameArchivePath('/home/me/a.zip/', '/home/me/a.zip')).toBe(true);
    expect(sameArchivePath('/home/me/a.zip', '/home/me/A.zip')).toBe(false);
    expect(sameArchivePath('/home/me/a.zip', '/home/me/b.zip')).toBe(false);
  });
});
