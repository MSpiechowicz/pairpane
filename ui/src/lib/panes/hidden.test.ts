import { render } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { describe, expect, it } from 'vitest';

import { Preferences } from '$lib/shell/preferences.svelte';

import { arrive, fakeBackend } from '../../../tests/panes';
import WatchHost from '../../../tests/WatchHost.svelte';
import { followPreferences } from './hidden.svelte';
import { PaneTabs } from './tabs.svelte';

describe('followPreferences', () => {
  it('lists both panes again when the preference changes, with or without hidden files', async () => {
    const backend = fakeBackend();
    const preferences = new Preferences();
    const hooks = { showHidden: () => preferences.value.showHidden };
    const panes = { left: new PaneTabs(backend.ipc, 'left', hooks), right: new PaneTabs(backend.ipc, 'right', hooks) };
    await panes.left.view.listing.open('/left');
    await panes.right.view.listing.open('/right');
    backend.opens.forEach((open, index) => arrive(open, index === 0 ? '/left' : '/right'));
    render(WatchHost, { start: () => followPreferences(panes, preferences) });
    flushSync();
    expect(backend.opens).toHaveLength(2); // nothing changed: nothing listed again

    preferences.set({ showHidden: true });
    flushSync();

    expect(backend.opens.slice(2).map((open) => [open.args.pane, open.args.path, open.args.showHidden])).toEqual([
      ['left', '/left', true],
      ['right', '/right', true],
    ]);

    preferences.set({ showHidden: false });
    flushSync();
    expect(backend.opens.slice(4).map((open) => open.args.showHidden)).toEqual([false, false]);
  });

  it('sorts a pane by name when the column it is sorted by is hidden, also when a tab brings that sort back', () => {
    const backend = fakeBackend();
    const preferences = new Preferences();
    const panes = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
    panes.left.view.sortBy('size');
    panes.right.view.sortBy('modified');
    render(WatchHost, { start: () => followPreferences(panes, preferences) });
    flushSync();

    preferences.set({ showSize: false });
    flushSync();
    expect([panes.left.view.column, panes.right.view.column]).toEqual(['name', 'modified']);

    panes.left.view.restoreSort('size', true);
    flushSync();
    expect(panes.left.view.column).toBe('name');
  });

  it('does the same for the extra columns, which are off until the settings show them', () => {
    const backend = fakeBackend();
    const preferences = new Preferences();
    const panes = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
    preferences.set({ showDimensions: true, showChecksum: true });
    panes.left.view.restoreSort('dimensions', true);
    panes.right.view.restoreSort('checksum', false);
    render(WatchHost, { start: () => followPreferences(panes, preferences) });
    flushSync();
    expect([panes.left.view.column, panes.right.view.column]).toEqual(['dimensions', 'checksum']);

    preferences.set({ showDimensions: false });
    flushSync();
    expect([panes.left.view.column, panes.right.view.column]).toEqual(['name', 'checksum']);

    preferences.set({ showDimensions: true });
    flushSync();
    expect([panes.left.view.column, panes.left.view.desc]).toEqual(['dimensions', true]);
  });

  it('brings the sort back when its column is shown again, unless the pane was sorted otherwise meanwhile', () => {
    const backend = fakeBackend();
    const preferences = new Preferences();
    const panes = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
    panes.left.view.restoreSort('size', true);
    panes.right.view.restoreSort('size', false);
    render(WatchHost, { start: () => followPreferences(panes, preferences) });
    flushSync();

    preferences.set({ showSize: false });
    flushSync();
    panes.right.view.sortBy('ext');
    flushSync();
    preferences.set({ showSize: true });
    flushSync();

    expect([panes.left.view.column, panes.left.view.desc]).toEqual(['size', true]);
    expect(panes.right.view.column).toBe('ext');
  });
});
