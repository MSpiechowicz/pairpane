import { beforeEach, describe, expect, it } from 'vitest';

import { canEncrypt, levelOr, levelsFor, PACK_DEFAULTS, packPath, rememberedPack, rememberPack, swapExtension } from './pack';

describe('packPath', () => {
  it('names a single item after it, in the folder given', () => {
    expect(packPath('/backup', ['photos'], '/home/ann', 'zip')).toBe('/backup/photos.zip');
    expect(packPath('/backup', ['notes.txt'], '/home/ann', 'sevenZ')).toBe('/backup/notes.txt.7z');
  });

  it('names several items after the folder they are in', () => {
    expect(packPath('/backup', ['a', 'b'], '/home/ann/docs', 'tarZst')).toBe('/backup/docs.tar.zst');
  });

  it('falls back to a plain name at a root, and keeps the separator of a Windows folder', () => {
    expect(packPath('D:\\out', ['a', 'b'], 'C:\\', 'zip')).toBe('D:\\out\\archive.zip');
    expect(packPath('/', ['a', 'b'], '/', 'zip')).toBe('/archive.zip');
  });
});

describe('swapExtension', () => {
  it('swaps the extension of a format for the one of another, compound .tar.zst included', () => {
    expect(swapExtension('/b/x.zip', 'tarZst')).toBe('/b/x.tar.zst');
    expect(swapExtension('/b/x.tar.zst', 'sevenZ')).toBe('/b/x.7z');
    expect(swapExtension('/b/x.7z', 'zip')).toBe('/b/x.zip');
    expect(swapExtension('/b/X.ZIP', 'sevenZ')).toBe('/b/X.7z');
  });

  it('leaves a name that ends with no known extension alone', () => {
    expect(swapExtension('/b/x', 'zip')).toBe('/b/x');
    expect(swapExtension('/b/x.tar', 'zip')).toBe('/b/x.tar');
    expect(swapExtension('/b/x.zip.bak', 'sevenZ')).toBe('/b/x.zip.bak');
  });
});

describe('levels and passwords per format', () => {
  it('a tar.zst has no Store; the others have all four levels', () => {
    expect(levelsFor('tarZst').map(({ level }) => level)).toEqual(['fast', 'normal', 'maximum']);
    expect(levelsFor('zip').map(({ label }) => label)).toEqual(['Store', 'Fast', 'Normal', 'Maximum']);
    expect(levelsFor('sevenZ')).toHaveLength(4);
  });

  it('a level the format lacks becomes Normal', () => {
    expect(levelOr('tarZst', 'store')).toBe('normal');
    expect(levelOr('tarZst', 'maximum')).toBe('maximum');
    expect(levelOr('zip', 'store')).toBe('store');
  });

  it('a tar.zst cannot be sealed with a password', () => {
    expect([canEncrypt('zip'), canEncrypt('sevenZ'), canEncrypt('tarZst')]).toEqual([true, true, false]);
  });
});

describe('the choice kept for the session', () => {
  beforeEach(() => rememberPack(PACK_DEFAULTS));

  it('starts at zip, Normal, names encrypted', () => {
    expect(rememberedPack()).toEqual({ format: 'zip', level: 'normal', encryptNames: true });
  });

  it('gives back the last choice', () => {
    rememberPack({ format: 'sevenZ', level: 'maximum', encryptNames: false });

    expect(rememberedPack()).toEqual({ format: 'sevenZ', level: 'maximum', encryptNames: false });
  });

  it('a remembered Store falls back to Normal for a tar.zst', () => {
    rememberPack({ format: 'tarZst', level: 'store', encryptNames: true });

    expect(rememberedPack().level).toBe('normal');
  });
});
