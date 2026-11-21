import { describe, expect, it } from 'vitest';

import {
  agreed,
  checksumName,
  customPart,
  formatMode,
  formatStamp,
  MAX_PARTS,
  modeMasks,
  PART_PRESETS,
  parseStamp,
  partCount,
  startModes,
} from './jobs';

describe('checksumName', () => {
  it('is the item`s name and the algorithm for one item', () => {
    expect(checksumName(['a.iso'], '/data', 'sha256')).toBe('a.iso.sha256');
    expect(checksumName(['notes'], '/data', 'blake3')).toBe('notes.blake3');
    expect(checksumName(['x'], '/data', 'md5')).toBe('x.md5');
  });

  it('is the folder`s name for several items, and CHECKSUM at a root', () => {
    expect(checksumName(['a', 'b'], '/home/ann/photos', 'sha256')).toBe('photos.sha256');
    expect(checksumName(['a', 'b'], 'C:\\Users\\ann\\photos\\', 'md5')).toBe('photos.md5');
    expect(checksumName(['a', 'b'], '/', 'sha256')).toBe('CHECKSUM.sha256');
    expect(checksumName(['a', 'b'], 'C:\\', 'blake3')).toBe('CHECKSUM.blake3');
  });
});

describe('part sizes', () => {
  it('offers the floppy, Zip, CD and FAT32 sizes', () => {
    expect(PART_PRESETS.map((preset) => preset.bytes)).toEqual([1_457_664, 104_857_600, 734_003_200, 4_294_967_295]);
  });

  it('turns an amount and a unit into bytes, and refuses less than 1 KB or what is no number', () => {
    expect(customPart(1.5, 1024)).toBe(1536);
    expect(customPart(2, 1024 ** 2)).toBe(2_097_152);
    expect(customPart(0.5, 1024)).toBeUndefined();
    expect(customPart(0, 1024)).toBeUndefined();
    expect(customPart(Number.NaN, 1024)).toBeUndefined();
    expect(customPart(1e300, 1024 ** 3)).toBeUndefined();
  });

  it('counts the parts, at least one', () => {
    expect(partCount(0, 1024)).toBe(1);
    expect(partCount(1024, 1024)).toBe(1);
    expect(partCount(1025, 1024)).toBe(2);
    expect(MAX_PARTS).toBe(999);
  });
});

describe('formatMode', () => {
  it('shows the octal and the letters', () => {
    expect(formatMode(0o755)).toBe('0755 rwxr-xr-x');
    expect(formatMode(0o644)).toBe('0644 rw-r--r--');
    expect(formatMode(0)).toBe('0000 ---------');
  });

  it('shows the special bits as ls does, upper case where the execute bit is missing', () => {
    expect(formatMode(0o4755)).toBe('4755 rwsr-xr-x');
    expect(formatMode(0o2755)).toBe('2755 rwxr-sr-x');
    expect(formatMode(0o1777)).toBe('1777 rwxrwxrwt');
    expect(formatMode(0o4644)).toBe('4644 rwSr--r--');
    expect(formatMode(0o1666)).toBe('1666 rw-rw-rwT');
  });

  it('ignores the file type bits a raw mode may carry', () => {
    expect(formatMode(0o100644)).toBe('0644 rw-r--r--');
  });
});

describe('permission states', () => {
  it('a bit is set, clear or left alone by what the items have in common', () => {
    expect(agreed([true, true])).toBe('on');
    expect(agreed([false, false])).toBe('off');
    expect(agreed([true, false])).toBe('keep');

    const start = startModes([0o644, 0o755]);
    expect(start[0o400]).toBe('on');
    expect(start[0o200]).toBe('on');
    expect(start[0o100]).toBe('keep');
    expect(start[0o040]).toBe('on');
    expect(start[0o020]).toBe('off');
    expect(start[0o4000]).toBe('off');
  });

  it('only what the user changed is sent', () => {
    const start = startModes([0o644]);

    expect(modeMasks(start, { ...start }, false)).toEqual({ add: 0, remove: 0 });
    expect(modeMasks(start, { ...start, [0o100]: 'on', [0o020]: 'on', [0o004]: 'off' }, false)).toEqual({
      add: 0o120,
      remove: 0o004,
    });
    expect(modeMasks(start, { ...start, [0o100]: 'keep' }, false)).toEqual({ add: 0, remove: 0 });
  });

  it('everything that is not left alone is sent when the change goes into folders', () => {
    const start = startModes([0o644]);

    expect(modeMasks(start, { ...start }, true)).toEqual({
      add: 0o644,
      remove: 0o4000 | 0o2000 | 0o1000 | 0o100 | 0o010 | 0o001 | 0o020 | 0o002,
    });
    expect(modeMasks(start, { ...start, [0o400]: 'keep' }, true).add).toBe(0o244);
  });
});

describe('times', () => {
  it('are shown and read as local time with seconds', () => {
    const millis = new Date(2026, 10, 20, 7, 58, 12).getTime();

    expect(formatStamp(millis)).toBe('2026-11-20 07:58:12');
    expect(parseStamp('2026-11-20 07:58:12')).toBe(millis);
    expect(parseStamp(' 2026-11-20T07:58:12 ')).toBe(millis);
  });

  it('seconds are optional', () => {
    expect(parseStamp('2026-11-20 07:58')).toBe(new Date(2026, 10, 20, 7, 58, 0).getTime());
  });

  it('a time that does not exist is none', () => {
    expect(parseStamp('')).toBeUndefined();
    expect(parseStamp('yesterday')).toBeUndefined();
    expect(parseStamp('2026-13-01 00:00')).toBeUndefined();
    expect(parseStamp('2026-02-31 00:00')).toBeUndefined();
    expect(parseStamp('2026-11-20 24:00')).toBeUndefined();
    expect(parseStamp('2026-11-20 07:61')).toBeUndefined();
    expect(parseStamp('2026-11-20 07:58:75')).toBeUndefined();
  });
});
