import { describe, expect, it } from 'vitest';

import type { PaneTabs } from '$lib/panes/tabs.svelte';
import type { PaneView } from '$lib/panes/view.svelte';

import { namesOf, recentFolders } from './sources';

const tabs = (folder: string | undefined, recent: string[]) => ({ view: { listing: { folder } }, recent }) as unknown as PaneTabs;

describe('recentFolders', () => {
  it('gives the other pane`s folder, then the active pane`s history, then the other one`s, each once, without the folder shown', () => {
    const found = recentFolders([tabs('/a', ['/b', '/a', '/c']), tabs('/d', ['/c', '/e'])]);

    expect(found).toEqual(['/d', '/b', '/c', '/e']);
  });
});

describe('namesOf', () => {
  it('gives the names of the entries loaded, in order', () => {
    const view = { listing: { entries: [{ name: 'x' }, undefined, { name: 'y' }] } } as unknown as PaneView;

    expect(namesOf(view)).toEqual(['x', 'y']);
  });
});
