import { render } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { describe, expect, it, vi } from 'vitest';

import type { FindEvent } from '$lib/ipc/bindings/FindEvent';
import type { ListEvent } from '$lib/ipc/bindings/ListEvent';
import { Preferences } from '$lib/shell/preferences.svelte';

import { done } from '../../../tests/find';
import { arrive, fakeBackend } from '../../../tests/panes';
import { go } from '../../../tests/tabs';
import WatchHost from '../../../tests/WatchHost.svelte';
import { FlatView } from './flat.svelte';
import { PaneTabs } from './tabs.svelte';

type Search = { args: { onEvent: (event: FindEvent) => void; query: { root: string; criteria: Record<string, unknown> } } };

/** Lets what is on its way (an answer, an effect) arrive. */
const later = () => new Promise((resolve) => setTimeout(resolve, 0));

const escape = (target?: EventTarget) => {
  const event = new KeyboardEvent('keydown', { key: 'Escape', cancelable: true });
  Object.defineProperty(event, 'target', { value: target ?? document.body });
  return event;
};

/** The left pane in `/home/me`; searches are numbered 1, 2, ... and a pane that opens results shows them at once. */
async function setup(answers: Parameters<typeof fakeBackend>[0] = {}) {
  const searches: Search[] = [];
  const backend = fakeBackend({
    find_start: (args: Search['args']) => {
      searches.push({ args });
      return searches.length;
    },
    // The backend lists the results into the pane before it answers.
    find_open: (args: { id: number; onEvent: (event: ListEvent) => void }) => {
      args.onEvent({ kind: 'start', path: '/home/me', parent: '/home', crumbs: [], total: 2, results: args.id });
      args.onEvent({ kind: 'done', dirs: 0, files: 2, bytes: 0, free: null });
      return null;
    },
    ...answers,
  });
  const panes = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
  const say = vi.fn();
  // The preference as the app has it: a change is seen by the effects.
  const preferences = new Preferences();
  const hidden = {
    get show() {
      return preferences.value.showHidden;
    },
    set show(show: boolean) {
      preferences.set({ showHidden: show });
    },
  };
  const flat = new FlatView(backend.ipc, panes, say, () => hidden.show);
  render(WatchHost, { start: () => flat.watch() });
  await go(backend, panes.left, '/home/me');

  const sent = (name: string) => backend.invoke.mock.calls.filter(([command]) => command === name).map(([, args]) => args);
  const listing = panes.left.view.listing;

  /** Starts the walk and lets its id arrive. */
  const walk = async () => {
    flat.left.toggle();
    await later();
  };

  return { backend, panes, listing, flat, say, hidden, searches, sent, walk };
}

describe('FlatView', () => {
  it('searches every file below the folder: no size limit that leaves files out, hidden ones as the pane shows them', async () => {
    const { flat, searches, hidden, walk } = await setup();

    await walk();

    expect(searches[0].args.query.root).toBe('/home/me');
    expect(searches[0].args.query.criteria).toMatchObject({ names: '', text: null, minSize: 0, maxSize: null, skipHidden: true, archives: false, duplicates: false });
    expect(flat.left.running).toBe(true);

    flat.left.stop();
    hidden.show = true;
    searches[0].args.onEvent(done(0, { cancelled: true }));
    await later();
    await walk();

    expect(searches[1].args.query.criteria).toMatchObject({ skipHidden: false });
  });

  it('lists the results in the pane when the walk is done, and says nothing more', async () => {
    const { flat, listing, say, sent, searches, walk } = await setup();
    await walk();

    searches[0].args.onEvent({ kind: 'found', hits: [] });
    searches[0].args.onEvent(done(2));
    await later();

    expect(sent('find_open')).toMatchObject([{ pane: 'left', id: 1 }]);
    expect(listing.results).toBe(1);
    expect(flat.left.running).toBe(false);
    expect(flat.left.shown).toBe(true);
    expect(flat.right.shown).toBe(false);
    expect(say).not.toHaveBeenCalled();
  });

  it('lists them also when the walk is done before its id is known', async () => {
    let answer: (id: number) => void = () => {};
    const { listing, searches, sent, flat } = await setup({
      find_start: ((args: Search['args']) => {
        searches.push({ args });
        return new Promise<number>((resolve) => (answer = resolve));
      }) as never,
    });
    flat.left.toggle();
    await later();

    searches[0].args.onEvent(done(2));
    await later();
    expect(sent('find_open')).toEqual([]);

    answer(7);
    await later();

    expect(sent('find_open')).toMatchObject([{ id: 7 }]);
    expect(listing.results).toBe(7);
  });

  it('says when it listed only the first files of a huge tree', async () => {
    const { say, searches, walk } = await setup();
    await walk();

    searches[0].args.onEvent(done(100000, { truncated: true }));
    await later();

    expect(say).toHaveBeenCalledWith('Only the first 100 000 files are listed.', false);
  });

  it('says how many items could not be read, and so are not listed', async () => {
    const { say, searches, walk } = await setup();
    await walk();

    searches[0].args.onEvent(done(2, { skipped: 3 }));
    await later();

    expect(say).toHaveBeenCalledWith('3 items could not be read and are left out.', false);
  });

  it('is made again when hidden files are shown or hidden while it is shown, of the same folder', async () => {
    const { flat, hidden, searches, walk } = await setup();
    await walk();
    searches[0].args.onEvent(done(2));
    await later();
    expect(flat.left.shown).toBe(true);

    hidden.show = true;
    flushSync();
    await later();

    expect(searches).toHaveLength(2);
    expect(searches[1].args.query).toMatchObject({ root: '/home/me', criteria: { skipHidden: false } });
  });

  it('leaves alone an Esc that something else used (a popup it closed)', async () => {
    const { flat, walk } = await setup();
    await walk();
    const used = escape();
    used.preventDefault();

    flat.onKey(used, 'left');

    expect(flat.left.running).toBe(true);
  });

  it('is stopped by the same key, and lists what was found so far, saying so', async () => {
    const { flat, listing, say, sent, searches, walk } = await setup();
    await walk();

    flat.left.toggle();
    flat.left.toggle(); // asking twice stops once

    expect(sent('find_cancel')).toEqual([{ id: 1 }]);
    expect(flat.left.running).toBe(true); // until the search says it ended

    searches[0].args.onEvent(done(2, { cancelled: true }));
    await later();

    expect(listing.results).toBe(1);
    expect(say).toHaveBeenCalledWith('Stopped: the 2 files found so far are listed.', false);
  });

  it('stopped before it found anything leaves the folder as it is and drops the search', async () => {
    const { flat, listing, say, sent, searches, walk } = await setup();
    await walk();

    flat.left.toggle();
    searches[0].args.onEvent(done(0, { cancelled: true }));
    await later();

    expect(sent('find_open')).toEqual([]);
    expect(listing.path).toBe('/home/me');
    expect(sent('find_forget')).toEqual([{ id: 1 }]);
    expect(say).toHaveBeenCalledWith('Stopped before any file was found.', false);
    expect(flat.left.running).toBe(false);
  });

  it('a stop asked before the id is known is sent once it is', async () => {
    let answer: (id: number) => void = () => {};
    const { flat, searches, sent } = await setup({
      find_start: ((args: Search['args']) => {
        searches.push({ args });
        return new Promise<number>((resolve) => (answer = resolve));
      }) as never,
    });
    flat.left.toggle();
    await later();

    flat.left.toggle();
    expect(sent('find_cancel')).toEqual([]);

    answer(4);
    await later();

    expect(sent('find_cancel')).toEqual([{ id: 4 }]);
  });

  it('Esc stops the walk of the active pane, and is left alone when nothing runs or a text field has it', async () => {
    const { flat, sent, walk } = await setup();

    const idle = escape();
    flat.onKey(idle, 'left');
    expect(idle.defaultPrevented).toBe(false);

    await walk();
    const typing = escape(document.createElement('input'));
    document.body.append(typing.target as Node);
    flat.onKey(typing, 'left');
    flat.onKey(escape(), 'right');
    expect(sent('find_cancel')).toEqual([]);

    const stopping = escape();
    flat.onKey(stopping, 'left');

    expect(stopping.defaultPrevented).toBe(true);
    expect(sent('find_cancel')).toEqual([{ id: 1 }]);
  });

  it('goes back to the folder from its results, and forgets the search once the folder is listed', async () => {
    const { backend, flat, listing, sent, searches, walk } = await setup();
    await walk();
    searches[0].args.onEvent(done(2));
    await later();

    flat.left.toggle();
    await later();
    expect(backend.last().args).toMatchObject({ pane: 'left', path: '/home/me' });
    expect(sent('find_forget')).toEqual([]); // the results stay until the folder is there
    arrive(backend.last(), '/home/me');
    flushSync();
    await later();

    expect(listing.results).toBeUndefined();
    expect(flat.left.shown).toBe(false);
    expect(sent('find_forget')).toEqual([{ id: 1 }]);
  });

  it('forgets the search when the pane lists another folder, and does not touch the others', async () => {
    const { backend, panes, flat, sent, searches, walk } = await setup();
    await walk();
    searches[0].args.onEvent(done(2));
    await later();

    await go(backend, panes.left, '/home/you');
    flushSync();
    await later();

    expect(sent('find_forget')).toEqual([{ id: 1 }]);
    expect(sent('find_cancel')).toEqual([]);

    // A new walk is another search, and the pane lists it.
    await walk();
    searches[1].args.onEvent(done(2));
    await later();
    expect(flat.left.shown).toBe(true);
    expect(sent('find_forget')).toEqual([{ id: 1 }]);
  });

  it('a pane that moves on while the walk runs drops it: its end lists nothing', async () => {
    const { backend, panes, listing, sent, searches, walk } = await setup();
    await walk();

    await listing.reload(); // the same folder again is not moving on
    arrive(backend.last(), '/home/me');
    flushSync();
    expect(sent('find_forget')).toEqual([]);

    await go(backend, panes.left, '/home/you');
    flushSync();
    await later();
    expect(sent('find_forget')).toEqual([{ id: 1 }]);

    searches[0].args.onEvent(done(2));
    await later();
    expect(sent('find_open')).toEqual([]);
    expect(listing.path).toBe('/home/you');
  });

  it('is refused in an archive and among search results, and says why', async () => {
    const { backend, panes, listing, flat, say, searches } = await setup();

    await panes.left.view.listing.openArchive({ archive: '/home/me/a.zip', dir: '' });
    backend.last().emit({ kind: 'start', path: '/home/me', parent: null, crumbs: [], total: 0, archive: { archive: '/home/me/a.zip', dir: '' } });
    flat.left.toggle();
    expect(say).toHaveBeenLastCalledWith('A flat view lists a folder on disk: leave the archive first.', true);

    await listing.openResults(9);
    flat.left.toggle();
    expect(say).toHaveBeenLastCalledWith('A flat view lists a folder, not search results: open the folder first.', true);

    expect(searches).toEqual([]);
    expect(flat.left.running).toBe(false);
  });

  it('says so when there are no files below the folder, and when the search fails', async () => {
    const { flat, say, sent, searches, walk } = await setup();
    await walk();

    searches[0].args.onEvent(done(0));
    await later();
    expect(say).toHaveBeenLastCalledWith('There are no files below this folder.', false);
    expect(sent('find_open')).toEqual([]);

    await walk();
    searches[1].args.onEvent({ kind: 'error', error: { code: 'io', message: 'Too many searches are running.' } });
    await later();

    expect(say).toHaveBeenLastCalledWith('Too many searches are running.', true);
    expect(sent('find_forget')).toEqual([{ id: 1 }, { id: 2 }]);
    expect(flat.left.running).toBe(false);
  });

  it('says when the search cannot be started', async () => {
    const { flat, say, sent } = await setup({
      find_start: (() => Promise.reject({ code: 'io', message: 'No search can start.' })) as never,
    });

    flat.left.toggle();
    await later();

    expect(say).toHaveBeenCalledWith('No search can start.', true);
    expect(flat.left.running).toBe(false);
    expect(sent('find_forget')).toEqual([]);
  });
});
