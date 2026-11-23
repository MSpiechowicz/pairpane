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
  it('gives the names the pane shows, in its order: `..` and the entries not loaded left out', () => {
    const rows = [undefined, { name: 'y' }, undefined, { name: 'x' }];
    const view = { count: rows.length, entryAt: (row: number) => rows[row] } as unknown as PaneView;

    expect(namesOf(view)).toEqual(['y', 'x']);
  });
});
