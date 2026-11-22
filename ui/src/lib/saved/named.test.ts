import { describe, expect, it } from 'vitest';

import { blankRule } from '$lib/rename/rule';
import { SavedRenames } from '$lib/rename/saved.svelte';

import { MAX_SAVED, MAX_SAVED_NAME } from './named.svelte';

describe('SavedRenames', () => {
  it('keeps presets in the order they were saved, under their trimmed names, and gives the rule back', () => {
    const saved = new SavedRenames();
    const rule = { ...blankRule(), name: '[YMD] [N]' };

    expect(saved.save(' Dated ', rule)).toBeUndefined();
    expect(saved.save('Plain', blankRule())).toBeUndefined();

    expect(saved.list.map((preset) => preset.name)).toEqual(['Dated', 'Plain']);
    expect(saved.valueFor('Dated')).toEqual(rule);
    expect(saved.valueFor('Missing')).toBeUndefined();
  });

  it('refuses an empty name and one that is too long, naming the preset in the message', () => {
    const saved = new SavedRenames();

    expect(saved.save('  ', blankRule())).toBe('Type a name for the preset.');
    expect(saved.save('x'.repeat(MAX_SAVED_NAME + 1), blankRule())).toMatch(/at most 64 characters/);
    expect(saved.save('é'.repeat(MAX_SAVED_NAME), blankRule())).toBeUndefined();
  });

  it('a name that is taken, in any case, is the same preset: it is replaced where it was', () => {
    const saved = new SavedRenames();
    saved.save('Dated', blankRule());
    saved.save('Other', blankRule());

    expect(saved.save('DATED', { ...blankRule(), name: 'x' })).toBeUndefined();

    expect(saved.list.map((preset) => preset.name)).toEqual(['DATED', 'Other']);
    expect(saved.list[0].rule.name).toBe('x');
  });

  it('keeps at most 50, but a full list can still replace one of its own', () => {
    const saved = new SavedRenames();
    for (let n = 0; n < MAX_SAVED; n += 1) {
      saved.save(`p${n}`, blankRule());
    }

    expect(saved.save('one more', blankRule())).toMatch(/Up to 50 presets/);
    expect(saved.save('P7', blankRule())).toBeUndefined();
    expect(saved.list).toHaveLength(MAX_SAVED);
  });

  it('removes by name, and takes over the presets of an earlier session (no more than 50)', () => {
    const saved = new SavedRenames();
    saved.restore(Array.from({ length: 60 }, (_, n) => ({ name: `p${n}`, rule: blankRule() })));
    expect(saved.list).toHaveLength(MAX_SAVED);

    saved.remove('p0');

    expect(saved.find('p0')).toBeUndefined();
    expect(saved.list).toHaveLength(MAX_SAVED - 1);
  });
});
