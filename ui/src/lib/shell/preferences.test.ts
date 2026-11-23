import { describe, expect, it } from 'vitest';

import { DEFAULT_PREFERENCES, Preferences } from './preferences.svelte';

describe('Preferences', () => {
  it('starts with the defaults: the columns of the listing shown, the extra ones not, hidden files not, every question asked, updates checked', () => {
    expect(new Preferences().value).toEqual({
      theme: 'system',
      showExt: true,
      showSize: true,
      showModified: true,
      showDimensions: false,
      showChecksum: false,
      showHidden: false,
      confirmTrash: true,
      confirmTransfer: true,
      confirmQuitWithTransfers: true,
      checkUpdatesAtStart: true,
    });
  });

  it('changes only what a patch names', () => {
    const preferences = new Preferences();

    preferences.set({ showSize: false });
    preferences.set({ theme: 'dark' });

    expect(preferences.value).toEqual({ ...DEFAULT_PREFERENCES, showSize: false, theme: 'dark' });
  });

  it('brings back what was saved', () => {
    const preferences = new Preferences();
    const saved = { ...DEFAULT_PREFERENCES, theme: 'light' as const, showHidden: true, confirmTrash: false, checkUpdatesAtStart: false };

    preferences.restore(saved);

    expect(preferences.value).toEqual(saved);
  });

  it('brings back the extra columns, and has them off for a session saved before they existed', () => {
    const preferences = new Preferences();

    preferences.restore({ showDimensions: true, showChecksum: true });
    expect(preferences.value).toMatchObject({ showDimensions: true, showChecksum: true });

    preferences.restore({ showExt: false });
    expect(preferences.value).toMatchObject({ showExt: false, showDimensions: false, showChecksum: false });
  });

  it('gives a default to what is saved wrongly (an unknown theme, a value that is no yes or no)', () => {
    const preferences = new Preferences();

    preferences.restore({ theme: 'neon', showExt: 'no', confirmTrash: false, showHidden: 1, showChecksum: 'yes' });

    expect(preferences.value).toEqual({ ...DEFAULT_PREFERENCES, confirmTrash: false });
  });

  it('has the defaults when nothing is saved, but keeps the theme the page shows already', () => {
    const preferences = new Preferences();
    preferences.set({ theme: 'dark', showExt: false, confirmTransfer: false });

    preferences.restore(undefined);

    expect(preferences.value).toEqual({ ...DEFAULT_PREFERENCES, theme: 'dark' });
  });
});
