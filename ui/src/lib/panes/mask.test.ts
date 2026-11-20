import { describe, expect, it } from 'vitest';

import { parseMask } from './mask';

describe('parseMask', () => {
  it('matches * and ? wildcards', () => {
    const match = parseMask('*.txt');
    expect(match('notes.txt')).toBe(true);
    expect(match('.txt')).toBe(true);
    expect(match('notes.txt.bak')).toBe(false);
    expect(match('txt')).toBe(false);

    expect(parseMask('a?c')('abc')).toBe(true);
    expect(parseMask('a?c')('ac')).toBe(false);
    expect(parseMask('a?c')('abbc')).toBe(false);
    expect(parseMask('a*b*c')('a-xx-b-yy-c')).toBe(true);
    expect(parseMask('a*b*c')('a-xx-c-yy-b')).toBe(false);
    expect(parseMask('*b')('abab')).toBe(true);
  });

  it('ignores case', () => {
    expect(parseMask('*.JPG')('photo.jpg')).toBe(true);
    expect(parseMask('read*')('README.md')).toBe(true);
  });

  it('joins masks with ; ignoring spaces and empty parts', () => {
    const match = parseMask(' *.png ;; *.JPG; ');
    expect(match('a.png')).toBe(true);
    expect(match('b.jpg')).toBe(true);
    expect(match('c.gif')).toBe(false);
  });

  it('treats *.* as everything, dot or not', () => {
    const match = parseMask('*.*');
    expect(match('Makefile')).toBe(true);
    expect(match('a.b')).toBe(true);
    expect(match('.hidden')).toBe(true);
    expect(parseMask('*.txt; *.*')('Makefile')).toBe(true);
  });

  it('matches nothing for an empty mask', () => {
    expect(parseMask('')('a')).toBe(false);
    expect(parseMask(' ; ')('a')).toBe(false);
    expect(parseMask('')('')).toBe(false);
  });

  it('takes regex characters literally', () => {
    expect(parseMask('[(.')('[(.')).toBe(true);
    expect(parseMask('[(.')('x')).toBe(false);
    expect(parseMask('a+b')('a+b')).toBe(true);
    expect(parseMask('a+b')('aab')).toBe(false);
    expect(parseMask('a.c')('abc')).toBe(false);
    expect(parseMask('(*)')('(x)')).toBe(true);
  });

  it('stays fast on a pathological pattern', () => {
    const match = parseMask('*a*a*a*a*a*a*a*a*b');
    const name = 'a'.repeat(50_000);

    const started = performance.now();
    expect(match(name)).toBe(false);
    expect(performance.now() - started).toBeLessThan(1000);
  });
});
