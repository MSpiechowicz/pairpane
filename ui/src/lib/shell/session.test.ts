import { render } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { blankForm, toCriteria } from '$lib/find/query';
import { SavedSearches } from '$lib/find/saved.svelte';
import type { FindCriteria } from '$lib/ipc/bindings/FindCriteria';
import type { PaneId } from '$lib/ipc/bindings/PaneId';
import type { SavedState } from '$lib/ipc/bindings/SavedState';
import { IpcFailure } from '$lib/ipc/transport';
import { Hotlist } from '$lib/panes/hotlist.svelte';
import { PaneTabs } from '$lib/panes/tabs.svelte';
import { blankRule } from '$lib/rename/rule';
import { SavedRenames } from '$lib/rename/saved.svelte';

import { arrive, fakeBackend } from '../../../tests/panes';
import SessionHost from '../../../tests/SessionHost.svelte';
import { BUSY_SAVE_MS, LOAD_TIMEOUT_MS, SAVE_DELAY_MS, Session } from './session.svelte';

const saved: SavedState = {
  active: 'right',
  left: {
    tabs: [
      { path: '/a', sort: 'size', desc: true },
      { path: '/b', sort: 'name', desc: false },
    ],
    active: 1,
  },
  right: { tabs: [{ path: '/r', sort: 'modified', desc: false }], active: 0 },
  hotlist: ['/fav'],
};

/** A session over a fake backend. `answers` decides what `state_load` and `state_save` do. */
function setup(answers: Parameters<typeof fakeBackend>[0] = {}) {
  const backend = fakeBackend(answers);
  const panes = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
  const hotlist = new Hotlist();
  const searches = new SavedSearches();
  const renames = new SavedRenames();
  const focus = { side: 'left' as PaneId };
  const session = new Session(backend.ipc, {
    panes,
    hotlist,
    searches,
    renames,
    active: { get: () => focus.side, set: (side) => (focus.side = side) },
  });

  const calls = (name: string) => backend.invoke.mock.calls.filter(([command]) => command === name);
  const saves = () => calls('state_save').map(([, args]) => (args as unknown as { state: SavedState }).state);

  /** Opens of one pane, as the paths they asked for. */
  const opened = (side: PaneId) => backend.opens.filter((open) => open.args.pane === side).map((open) => open.args.path);

  /** Lets the pane's latest open arrive as a folder. */
  const list = (side: PaneId) => {
    const open = backend.opens.filter((candidate) => candidate.args.pane === side).at(-1)!;
    arrive(open, (open.args.path as string | null) ?? '/home/me');
    flushSync();
  };

  return { backend, panes, hotlist, searches, renames, focus, session, calls, saves, opened, list };
}

type Setup = ReturnType<typeof setup>;

/** Restores, lists the folders and lets the effect see them. */
async function started(setupped: Setup) {
  render(SessionHost, { session: setupped.session });
  await setupped.session.restore();
  setupped.list('left');
  setupped.list('right');
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 2);
}

beforeEach(() => vi.useFakeTimers());

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('Session restore', () => {
  it('brings back the tabs, their folders, the active tabs and pane, and the hotlist', async () => {
    const s = setup({ state_load: () => saved });

    await s.session.restore();

    expect(s.opened('left')).toEqual(['/b']);
    expect(s.opened('right')).toEqual(['/r']);
    expect(s.panes.left.tabs).toHaveLength(2);
    expect(s.panes.left.active).toBe(1);
    expect(s.panes.left.state[0].sort).toEqual({ column: 'size', desc: true });
    expect(s.focus.side).toBe('right');
    expect(s.hotlist.paths).toEqual(['/fav']);
  });

  it('starts with the defaults when nothing is saved: both panes in the home folder', async () => {
    const s = setup();

    await s.session.restore();

    expect(s.opened('left')).toEqual([null]);
    expect(s.opened('right')).toEqual([null]);
    expect(s.focus.side).toBe('left');
    expect(s.hotlist.paths).toEqual([]);
  });

  it('uses the defaults for a pane that has no usable tab', async () => {
    const s = setup({ state_load: () => ({ ...saved, left: { tabs: [], active: 0 } }) });

    await s.session.restore();

    expect(s.opened('left')).toEqual([null]);
    expect(s.opened('right')).toEqual(['/r']);
  });

  it('starts with the defaults when the state cannot be loaded', async () => {
    const s = setup({ state_load: () => Promise.reject(new IpcFailure({ code: 'io', message: 'unreadable' })) });

    await s.session.restore();

    expect(s.opened('left')).toEqual([null]);
  });

  it('does not wait longer than the timeout for a load that hangs, and ignores it if it ever answers', async () => {
    let answer: (state: SavedState) => void = () => {};
    const s = setup({ state_load: () => new Promise((resolve) => (answer = resolve)) });

    const restoring = s.session.restore();
    await vi.advanceTimersByTimeAsync(LOAD_TIMEOUT_MS - 1);
    expect(s.opened('left')).toEqual([]);

    await vi.advanceTimersByTimeAsync(1);
    await restoring;
    expect(s.opened('left')).toEqual([null]);

    answer(saved);
    await vi.advanceTimersByTimeAsync(0);
    expect(s.opened('left')).toEqual([null]);
    expect(s.hotlist.paths).toEqual([]);
  });
});

describe('Session saving', () => {
  it('does not write what it has just restored', async () => {
    const s = setup({ state_load: () => saved });

    render(SessionHost, { session: s.session });
    await s.session.restore();
    s.list('left');
    s.list('right');
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 3);

    expect(s.saves()).toEqual([]);
  });

  it('never overwrites a state it could not load (refused, or too slow)', async () => {
    const hanging = () => new Promise<never>(() => {});
    const refused = () => Promise.reject(new IpcFailure({ code: 'busy', message: 'later' }));

    for (const state_load of [hanging, refused]) {
      const s = setup({ state_load });
      render(SessionHost, { session: s.session });
      const restoring = s.session.restore();
      await vi.advanceTimersByTimeAsync(LOAD_TIMEOUT_MS);
      await restoring;

      s.list('left');
      s.list('right');
      s.hotlist.add('/one');
      await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 3);
      window.dispatchEvent(new Event('pagehide'));

      expect(s.saves()).toEqual([]);
    }
  });

  it('writes nothing before the folders have been listed (the home folder has no path yet)', async () => {
    const s = setup();

    render(SessionHost, { session: s.session });
    await s.session.restore();
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 3);
    expect(s.saves()).toEqual([]);

    s.list('left');
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 3);
    expect(s.saves()).toEqual([]); // the right pane is still without one

    s.list('right');
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 3);
    expect(s.saves()).toHaveLength(1);
  });

  it('saves the home folder as the path the listing reported', async () => {
    const s = setup();
    await started(s);

    expect(s.saves()).toEqual([
      {
        active: 'left',
        left: { tabs: [{ path: '/home/me', sort: 'name', desc: false }], active: 0 },
        right: { tabs: [{ path: '/home/me', sort: 'name', desc: false }], active: 0 },
        hotlist: [],
        searches: [],
        renames: [],
      },
    ]);
  });

  it('brings back the saved searches, and every save carries them, a delete included', async () => {
    const criteria = toCriteria(blankForm(), false) as FindCriteria;
    const s = setup({ state_load: () => ({ ...saved, searches: [{ name: 'Big', criteria }] }) });
    await started(s);

    expect(s.searches.list).toEqual([{ name: 'Big', criteria }]);

    s.searches.save('Small', criteria);
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 2);
    expect(s.saves().at(-1)?.searches?.map((search) => search.name)).toEqual(['Big', 'Small']);

    s.searches.remove('Big');
    s.searches.remove('Small');
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 2);
    expect(s.saves().at(-1)?.searches).toEqual([]);
  });

  it('a state file from before saved searches loads with none', async () => {
    const s = setup({ state_load: () => saved });
    await started(s);

    expect(s.searches.list).toEqual([]);
    expect(s.renames.list).toEqual([]);
  });

  it('brings back the saved rename rules, and every save carries them, a delete included', async () => {
    const rule = blankRule();
    const s = setup({ state_load: () => ({ ...saved, renames: [{ name: 'Tidy', rule }] }) });
    await started(s);

    expect(s.renames.list).toEqual([{ name: 'Tidy', rule }]);

    s.renames.save('Dated', { ...rule, name: '[YMD] [N]' });
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 2);
    expect(s.saves().at(-1)?.renames?.map((preset) => preset.name)).toEqual(['Tidy', 'Dated']);

    s.renames.remove('Tidy');
    s.renames.remove('Dated');
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 2);
    expect(s.saves().at(-1)?.renames).toEqual([]);
  });

  it('a save while a tab`s folder is still being opened stores that folder, not the one the pane still shows', async () => {
    const s = setup();
    await started(s);
    s.panes.left.add();
    await s.panes.left.view.listing.open('/b');
    s.list('left');
    s.panes.left.select(0);
    s.list('left');

    s.panes.left.select(1); // /b is slow: the pane still shows the home folder
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 2);

    const left = s.saves().at(-1)?.left;
    expect(left?.active).toBe(1);
    expect(left?.tabs.map((tab) => tab.path)).toEqual(['/home/me', '/b']);
  });

  it('writes a burst of changes once, after the last one, with the state as it ended', async () => {
    const s = setup();
    await started(s);
    const before = s.saves().length;

    s.hotlist.add('/one');
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS - 1);
    s.hotlist.add('/two');
    s.focus.side = 'right';
    s.panes.left.view.sortBy('size');
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS - 1);
    expect(s.saves()).toHaveLength(before);

    await vi.advanceTimersByTimeAsync(1);
    expect(s.saves()).toHaveLength(before + 1);
    expect(s.saves().at(-1)).toMatchObject({
      hotlist: ['/one', '/two'],
      left: { tabs: [{ sort: 'size' }] },
    });
  });

  it('writes nothing when a change is undone before it is written', async () => {
    const s = setup();
    await started(s);
    const before = s.saves().length;

    s.hotlist.add('/x');
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS / 2);
    s.hotlist.remove('/x');
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 3);

    expect(s.saves()).toHaveLength(before);
  });

  it('does not keep history, only the folder each tab is in', async () => {
    const s = setup();
    await started(s);

    void s.panes.left.view.listing.open('/elsewhere');
    arrive(s.backend.opens.filter((open) => open.args.pane === 'left').at(-1)!, '/elsewhere');
    flushSync();
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);

    expect(s.saves().at(-1)?.left.tabs).toEqual([{ path: '/elsewhere', sort: 'name', desc: false }]);
    expect(JSON.stringify(s.saves().at(-1))).not.toContain('/home/me/');
  });

  it('asks again with the latest state when the backend is busy', async () => {
    let busy = true;
    const s = setup({
      state_save: () => (busy ? Promise.reject(new IpcFailure({ code: 'busy', message: 'later' })) : null),
    });
    await started(s);
    expect(s.saves()).toHaveLength(1);

    s.hotlist.add('/while-busy');
    busy = false;
    await vi.advanceTimersByTimeAsync(BUSY_SAVE_MS * 2);

    expect(s.saves().length).toBeGreaterThan(1);
    expect(s.saves().at(-1)?.hotlist).toEqual(['/while-busy']);

    const writes = s.saves().length;
    await vi.advanceTimersByTimeAsync(BUSY_SAVE_MS * 5);
    expect(s.saves()).toHaveLength(writes); // saved: no more asking
  });

  it('logs another failure once and does not retry it in a loop', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const s = setup({
      state_save: () => Promise.reject(new IpcFailure({ code: 'io', message: 'the disk is full' })),
    });
    await started(s);
    await vi.advanceTimersByTimeAsync(BUSY_SAVE_MS * 10);
    expect(s.saves()).toHaveLength(1);

    s.hotlist.add('/more');
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 2);

    expect(s.saves()).toHaveLength(2); // a new change is a new attempt, nothing else is
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it('writes a change at once when the page goes away', async () => {
    const s = setup();
    await started(s);
    const before = s.saves().length;

    s.hotlist.add('/last-minute');
    flushSync();
    window.dispatchEvent(new Event('pagehide'));

    expect(s.saves()).toHaveLength(before + 1);
    expect(s.saves().at(-1)?.hotlist).toEqual(['/last-minute']);
  });
});
