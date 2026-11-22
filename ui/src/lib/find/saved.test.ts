import { describe, expect, it } from 'vitest';

import type { FindCriteria } from '$lib/ipc/bindings/FindCriteria';

import { blankForm, toCriteria } from './query';
import { MAX_SEARCH_NAME, MAX_SEARCHES, SavedSearches } from './saved.svelte';

const criteria = (names = '') => toCriteria({ ...blankForm(), names }, false) as FindCriteria;

describe('SavedSearches', () => {
  it('keeps searches in the order they were saved, under their trimmed names', () => {
    const saved = new SavedSearches();

    expect(saved.save(' Logs ', criteria('*.log'))).toBeUndefined();
    expect(saved.save('Notes', criteria('*.md'))).toBeUndefined();

    expect(saved.list.map((search) => search.name)).toEqual(['Logs', 'Notes']);
    expect(saved.find('Logs')?.criteria.names).toBe('*.log');
  });

  it('refuses an empty name and one that is too long', () => {
    const saved = new SavedSearches();

    expect(saved.save('   ', criteria())).toBe('Type a name for the search.');
    expect(saved.save('x'.repeat(MAX_SEARCH_NAME + 1), criteria())).toMatch(/at most 64 characters/);
    expect(saved.save('é'.repeat(MAX_SEARCH_NAME), criteria())).toBeUndefined();
    expect(saved.list).toHaveLength(1);
  });

  it('a name that is taken, in any case, is the same search: it is replaced where it was', () => {
    const saved = new SavedSearches();
    saved.save('Logs', criteria('*.log'));
    saved.save('Other', criteria());

    expect(saved.save('LOGS', criteria('*.txt'))).toBeUndefined();

    expect(saved.list.map((search) => search.name)).toEqual(['LOGS', 'Other']);
    expect(saved.list[0].criteria.names).toBe('*.txt');
  });

  it('keeps at most 50, but a full list can still replace one of its own', () => {
    const saved = new SavedSearches();
    for (let n = 0; n < MAX_SEARCHES; n += 1) {
      saved.save(`s${n}`, criteria());
    }

    expect(saved.save('one more', criteria())).toMatch(/Up to 50 searches/);
    expect(saved.save('S7', criteria('x'))).toBeUndefined();
    expect(saved.list).toHaveLength(MAX_SEARCHES);
  });

  it('removes by name, and takes over the searches of an earlier session (no more than 50)', () => {
    const saved = new SavedSearches();
    saved.restore(Array.from({ length: 60 }, (_, n) => ({ name: `s${n}`, criteria: criteria() })));
    expect(saved.list).toHaveLength(MAX_SEARCHES);

    saved.remove('s0');

    expect(saved.find('s0')).toBeUndefined();
    expect(saved.list).toHaveLength(MAX_SEARCHES - 1);
  });
});
