import { describe, expect, it } from 'vitest';

import { formatEta, formatFacts, formatRate, formatSpeed, percentDone } from './format';

describe('formatSpeed', () => {
  it('is a size per second', () => {
    expect(formatSpeed(0)).toBe('0 B/s');
    expect(formatSpeed(1536)).toBe('1.5 KB/s');
    expect(formatSpeed(12 * 1024 * 1024)).toBe('12 MB/s');
  });
});

describe('formatEta', () => {
  it('uses the two biggest units, and seconds only below a minute', () => {
    expect(formatEta(0)).toBe('0 s');
    expect(formatEta(42)).toBe('42 s');
    expect(formatEta(60)).toBe('1 min');
    expect(formatEta(185)).toBe('3 min 5 s');
    expect(formatEta(3600)).toBe('1 h');
    expect(formatEta(7500)).toBe('2 h 5 min');
  });

  it('rounds to whole seconds and never goes negative', () => {
    expect(formatEta(41.6)).toBe('42 s');
    expect(formatEta(-3)).toBe('0 s');
  });
});

describe('formatRate', () => {
  it('joins speed and time left', () => {
    expect(formatRate({ bytesPerSec: 2048, etaSecs: 90 })).toBe('2.0 KB/s · 1 min 30 s left');
  });

  it('leaves out what is not known yet', () => {
    expect(formatRate({ bytesPerSec: 2048, etaSecs: null })).toBe('2.0 KB/s');
    expect(formatRate({ bytesPerSec: 0, etaSecs: 5 })).toBe('5 s left');
    expect(formatRate({ bytesPerSec: 0, etaSecs: null })).toBe('');
  });
});

describe('percentDone', () => {
  const counts = { doneBytes: 0, totalBytes: 0, doneFiles: 0, totalFiles: 0 };

  it('is the share of the bytes, rounded down and capped', () => {
    expect(percentDone({ ...counts, doneBytes: 45, totalBytes: 100 })).toBe(45);
    expect(percentDone({ ...counts, doneBytes: 2, totalBytes: 3 })).toBe(66);
    expect(percentDone({ ...counts, doneBytes: 120, totalBytes: 100 })).toBe(100);
  });

  it('falls back to the files when there are no bytes, and is 0 before anything is known', () => {
    expect(percentDone({ ...counts, doneFiles: 1, totalFiles: 4 })).toBe(25);
    expect(percentDone(counts)).toBe(0);
  });
});

describe('formatFacts', () => {
  it('is size and date for a file, and just the date for a folder`s size', () => {
    const modified = new Date(2026, 10, 20, 7, 58).getTime();

    expect(formatFacts({ size: 4096, modified, dir: false })).toBe('4.0 KB · 2026-11-20 07:58');
    expect(formatFacts({ size: 0, modified, dir: true })).toBe('Folder · 2026-11-20 07:58');
    expect(formatFacts({ size: 10, modified: null, dir: false })).toBe('10 B');
  });
});
