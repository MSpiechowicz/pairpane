import { describe, expect, it, vi } from 'vitest';

import { arrive, fakeBackend } from '../../../tests/panes';
import { arrived, asked, go, inA, shown } from '../../../tests/tabs';
import { MAX_HISTORY, MAX_TABS, PaneTabs } from './tabs.svelte';

describe('PaneTabs history', () => {
  it('back and forward walk the folders visited, in the active tab', async () => {
    const { backend, tabs } = await inA();
    await go(backend, tabs, '/b');
    await go(backend, tabs, '/c');
    expect(tabs.canBack).toBe(true);
    expect(tabs.canForward).toBe(false);

    tabs.back();
    expect(asked(backend)).toBe('/b');
    arrived(backend);
    tabs.back();
    expect(asked(backend)).toBe('/a');
    arrived(backend);
    expect(tabs.canBack).toBe(false);
    expect(tabs.canForward).toBe(true);

    tabs.forward();
    expect(asked(backend)).toBe('/b');
    arrived(backend);
    tabs.forward();
    expect(asked(backend)).toBe('/c');
    arrived(backend);
    expect(shown(tabs)).toBe('/c');
    expect(tabs.canForward).toBe(false);
  });

  it('walking the history does not add to it', async () => {
    const { backend, tabs } = await inA();
    await go(backend, tabs, '/b');

    tabs.back();
    arrived(backend);
    tabs.forward();
    arrived(backend);
    tabs.back();
    arrived(backend);

    expect(shown(tabs)).toBe('/a');
    expect(tabs.canBack).toBe(false);
    expect(tabs.canForward).toBe(true);
  });

  it('going somewhere new clears what was ahead', async () => {
    const { backend, tabs } = await inA();
    await go(backend, tabs, '/b');
    await go(backend, tabs, '/c');
    tabs.back();
    arrived(backend);
    expect(tabs.canForward).toBe(true);

    await go(backend, tabs, '/d');
    expect(tabs.canForward).toBe(false);

    tabs.back();
    expect(asked(backend)).toBe('/b');
  });

  it('listing the shown folder again (a change on disk, the same crumb) is not a step', async () => {
    const { backend, tabs } = await inA();

    await go(backend, tabs, '/a');
    await go(backend, tabs, '/a');

    expect(tabs.canBack).toBe(false);
  });

  it('holds at most 50 folders on each side, forgetting the oldest', async () => {
    const { backend, tabs } = await inA();
    for (let i = 1; i <= MAX_HISTORY + 10; i += 1) {
      await go(backend, tabs, `/f${i}`);
    }

    let steps = 0;
    while (tabs.canBack) {
      tabs.back();
      arrived(backend);
      steps += 1;
    }

    expect(steps).toBe(MAX_HISTORY);
    expect(shown(tabs)).toBe('/f10'); // /a and /f1…/f9 fell off the end

    let forward = 0;
    while (tabs.canForward) {
      tabs.forward();
      arrived(backend);
      forward += 1;
    }
    expect(forward).toBe(MAX_HISTORY);
    expect(shown(tabs)).toBe(`/f${MAX_HISTORY + 10}`);
  });

  it('a folder that is gone is dropped from the history; the pane stays where it is', async () => {
    const { backend, tabs } = await inA();
    await go(backend, tabs, '/b');
    await go(backend, tabs, '/c');

    tabs.back();
    backend.last().emit({ kind: 'error', error: { code: 'notFound', message: 'This folder does not exist.' } });
    expect(shown(tabs)).toBe('/c');
    expect(tabs.view.listing.notice).toBe('This folder does not exist.');
    expect(tabs.canForward).toBe(false);

    tabs.back();
    expect(asked(backend)).toBe('/a'); // /b is gone from the list
  });

  it('does nothing without history', async () => {
    const { backend, tabs } = await inA();
    const opens = backend.opens.length;

    tabs.back();
    tabs.forward();

    expect(backend.opens).toHaveLength(opens);
  });
});

describe('PaneTabs tabs', () => {
  it('starts with one tab; a new one shows the current folder and becomes the active one', async () => {
    const { backend, tabs } = await inA();
    expect(tabs.tabs).toHaveLength(1);

    expect(tabs.add()).toBe(true);

    expect(tabs.tabs).toHaveLength(2);
    expect(tabs.active).toBe(1);
    expect(tabs.state.map((tab) => tab.path)).toEqual(['/a', '/a']);
    expect(backend.opens).toHaveLength(1); // the same folder is already listed
  });

  it('keeps at most 32 tabs', async () => {
    const { tabs } = await inA();

    for (let i = 1; i < MAX_TABS; i += 1) {
      expect(tabs.add()).toBe(true);
    }

    expect(tabs.add()).toBe(false);
    expect(tabs.tabs).toHaveLength(MAX_TABS);
  });

  it('every tab keeps its own folder and sort; switching opens the folder and restores the sort', async () => {
    const { backend, tabs } = await inA();
    tabs.view.sortBy('size');
    tabs.add();
    await go(backend, tabs, '/b');
    tabs.view.sortBy('modified');
    tabs.view.sortBy('modified');
    expect(tabs.state).toEqual([
      { path: '/a', sort: { column: 'size', desc: false }, layout: 'detailed' },
      { path: '/b', sort: { column: 'modified', desc: true }, layout: 'detailed' },
    ]);

    tabs.select(0);
    expect(asked(backend)).toBe('/a');
    expect(tabs.view.column).toBe('size');
    expect(tabs.view.desc).toBe(false);
    arrived(backend);

    tabs.select(1);
    expect(asked(backend)).toBe('/b');
    expect(tabs.view.column).toBe('modified');
    expect(tabs.view.desc).toBe(true);
  });

  it('every tab keeps its own layout, and a new tab starts with the one of the tab it was made from', async () => {
    const { backend, tabs } = await inA();
    expect(tabs.view.layout).toBe('detailed');

    tabs.view.layout = 'brief';
    tabs.add();
    expect(tabs.view.layout).toBe('brief');
    await go(backend, tabs, '/b');
    tabs.view.layout = 'detailed';
    expect(tabs.state.map((tab) => tab.layout)).toEqual(['brief', 'detailed']);

    tabs.select(0);
    expect(tabs.view.layout).toBe('brief');
    arrived(backend);

    tabs.select(1);
    expect(tabs.view.layout).toBe('detailed');
    expect(tabs.state.map((tab) => tab.layout)).toEqual(['brief', 'detailed']);
  });

  it('restores the layouts it is given; a tab without one is shown in detail', async () => {
    const { tabs } = await inA();

    tabs.restore(
      [
        { path: '/a', sort: 'name', desc: false, layout: 'brief' },
        { path: '/b', sort: 'name', desc: false },
      ],
      0,
    );
    expect(tabs.view.layout).toBe('brief');
    expect(tabs.state.map((tab) => tab.layout)).toEqual(['brief', 'detailed']);

    tabs.select(1);
    expect(tabs.view.layout).toBe('detailed');
  });

  it('every tab keeps its own history, and switching is not a step', async () => {
    const { backend, tabs } = await inA();
    tabs.add();
    await go(backend, tabs, '/b');
    await go(backend, tabs, '/c'); // second tab: a → b → c

    tabs.select(0);
    arrived(backend);
    expect(tabs.canBack).toBe(false);

    tabs.select(1);
    arrived(backend); // /c
    expect(tabs.canBack).toBe(true);
    tabs.back();
    expect(asked(backend)).toBe('/b');
    arrived(backend);
    tabs.back();
    expect(asked(backend)).toBe('/a');
  });

  it('a tab`s folder is read live: a tab switched away from keeps where it had got to', async () => {
    const { backend, tabs } = await inA();
    tabs.add();
    tabs.select(0);
    arrived(backend);
    await go(backend, tabs, '/elsewhere');

    tabs.select(1);
    arrived(backend);
    tabs.select(0);

    expect(asked(backend)).toBe('/elsewhere');
  });

  it('closing the active tab shows its right neighbour (the left one at the end); the last tab stays', async () => {
    const { backend, tabs } = await inA();
    tabs.add();
    await go(backend, tabs, '/b');
    tabs.add();
    await go(backend, tabs, '/c');
    tabs.select(1);
    arrived(backend);

    expect(tabs.close()).toBe(true);
    expect(tabs.active).toBe(1);
    expect(asked(backend)).toBe('/c');
    arrived(backend);

    expect(tabs.close()).toBe(true);
    expect(tabs.active).toBe(0);
    expect(asked(backend)).toBe('/a');
    arrived(backend);

    expect(tabs.close()).toBe(false);
    expect(tabs.tabs).toHaveLength(1);
  });

  it('closing another tab leaves the active one, and the pane, alone', async () => {
    const { backend, tabs } = await inA();
    tabs.add();
    tabs.add();
    const opens = backend.opens.length;
    const id = tabs.tabs[2].id;

    tabs.close(0);

    expect(tabs.tabs).toHaveLength(2);
    expect(tabs.tabs[tabs.active].id).toBe(id);
    expect(backend.opens).toHaveLength(opens);

    tabs.close(5);
    expect(tabs.tabs).toHaveLength(2);
  });

  it('cycles around both ends', async () => {
    const { backend, tabs } = await inA();
    tabs.add();
    tabs.add();
    expect(tabs.active).toBe(2);

    tabs.cycle(1);
    expect(tabs.active).toBe(0);
    arrived(backend);

    tabs.cycle(-1);
    expect(tabs.active).toBe(2);
    arrived(backend);
    tabs.cycle(-1);
    expect(tabs.active).toBe(1);
  });

  it('a lone tab does not reopen itself', async () => {
    const { backend, tabs } = await inA();
    const opens = backend.opens.length;

    tabs.cycle(1);
    tabs.select(0);

    expect(backend.opens).toHaveLength(opens);
  });
});

describe('PaneTabs restore', () => {
  const saved = [
    { path: '/a', sort: 'size' as const, desc: true },
    { path: '/b', sort: 'name' as const, desc: false },
    { path: '/c', sort: 'modified' as const, desc: false },
  ];

  it('brings back the tabs with their folders and sorts, and shows the active one', () => {
    const backend = fakeBackend();
    const tabs = new PaneTabs(backend.ipc, 'left');

    tabs.restore(saved, 1);

    expect(tabs.tabs).toHaveLength(3);
    expect(tabs.active).toBe(1);
    expect(asked(backend)).toBe('/b');
    expect(tabs.state.map((tab) => tab.path)).toEqual(['/a', '/b', '/c']);
    expect(tabs.state[0].sort).toEqual({ column: 'size', desc: true });

    arrived(backend);
    tabs.select(0);
    expect(asked(backend)).toBe('/a');
    expect(tabs.view.column).toBe('size');
    expect(tabs.view.desc).toBe(true);
  });

  it('starts with no history, and the first folder is not a step of it', () => {
    const backend = fakeBackend();
    const tabs = new PaneTabs(backend.ipc, 'left');

    tabs.restore(saved, 0);
    arrived(backend);

    expect(tabs.canBack).toBe(false);
    expect(tabs.canForward).toBe(false);
  });

  it('nothing saved means the defaults: one tab in the home folder', () => {
    const backend = fakeBackend();
    const tabs = new PaneTabs(backend.ipc, 'left');

    tabs.restore([], 0);

    expect(tabs.tabs).toHaveLength(1);
    expect(asked(backend)).toBeNull();
  });

  it('keeps an out-of-range active tab and an oversized list within bounds', () => {
    const backend = fakeBackend();
    const tabs = new PaneTabs(backend.ipc, 'left');
    const many = Array.from({ length: MAX_TABS + 5 }, (_, index) => ({ path: `/f${index}`, sort: 'name' as const, desc: false }));

    tabs.restore(many, 999);

    expect(tabs.tabs).toHaveLength(MAX_TABS);
    expect(tabs.active).toBe(MAX_TABS - 1);
  });

  it('falls back to the home folder, once, when the restored folder cannot be opened', () => {
    const backend = fakeBackend();
    const onFail = vi.fn();
    const tabs = new PaneTabs(backend.ipc, 'left', { onFail });
    const refuse = () =>
      backend.last().emit({ kind: 'error', error: { code: 'notFound', message: 'No such folder.' } });

    tabs.restore(saved, 0);
    refuse();
    expect(asked(backend)).toBeNull();

    refuse(); // the home folder fails as well: no loop
    expect(backend.opens).toHaveLength(2);
    expect(onFail).toHaveBeenCalledTimes(2);
  });

  it('does not fall back for a folder that failed partway: it was shown, so it is the user`s to deal with', () => {
    const backend = fakeBackend();
    const tabs = new PaneTabs(backend.ipc, 'left');

    tabs.restore(saved, 0);
    backend.last().emit({ kind: 'start', path: '/a', parent: '/', crumbs: [], total: 10 });
    backend.last().emit({ kind: 'error', error: { code: 'io', message: 'The disk went away.' } });

    expect(backend.opens).toHaveLength(1);
  });
});
