import { describe, expect, it } from 'vitest';

import { score } from './fuzzy';

describe('score', () => {
  it('matches the letters in order, whatever their case, and nothing out of order', () => {
    expect(score('cpy', 'Copy')).toBeDefined();
    expect(score('COPY', 'copy')).toBeDefined();
    expect(score('ypc', 'Copy')).toBeUndefined();
    expect(score('x', 'Copy')).toBeUndefined();
  });

  it('ranks the starts of words and letters that follow one another above scattered ones', () => {
    const atStarts = score('nf', 'New folder')!;
    const scattered = score('nf', 'Synchronize folders…')!;
    expect(atStarts).toBeGreaterThan(scattered);

    const together = score('move', 'Move')!;
    const apart = score('move', 'Multi-rename over everything')!;
    expect(together).toBeGreaterThan(apart);
  });

  it('matches everything with nothing typed, and spaces in the query are skipped', () => {
    expect(score('', 'anything')).toBe(0);
    expect(score('new tab', 'New tab')).toBeDefined();
  });
});
