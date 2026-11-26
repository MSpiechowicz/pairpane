import { fireEvent, screen, waitFor, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import type { FindEvent } from '$lib/ipc/bindings/FindEvent';
import type { ListEvent } from '$lib/ipc/bindings/ListEvent';
import { DEFAULT_PREFERENCES, preferences } from '$lib/shell/preferences.svelte';

import { appShown, key, listbox, pane, withArchiveOnRight } from '../tests/app';
import { done } from '../tests/find';
import { crumbsTo, entry, resultsStart } from '../tests/panes';

/** The walks the page started (their channels), and a pane that lists the results of search `id` as the backend does. */
function walking() {
  const started: { args: { query: { root: string; criteria: Record<string, unknown> } }; emit: (event: FindEvent) => void }[] = [];
  const answers = {
    find_start: (args: { onEvent: (event: FindEvent) => void; query: { root: string; criteria: Record<string, unknown> } }) => {
      started.push({ args, emit: args.onEvent });
      return started.length;
    },
    find_open: (args: { id: number; onEvent: (event: ListEvent) => void }) => {
      args.onEvent(resultsStart('/left', 2, args.id, { crumbs: crumbsTo('/left') }));
      args.onEvent({ kind: 'batch', offset: 0, entries: [entry('docs/a.txt'), entry('b.txt')] });
      args.onEvent({ kind: 'done', dirs: 0, files: 2, bytes: 0, free: null });
      return null;
    },
  };
  return { started, answers };
}

const left = () => within(pane('Left pane'));

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
  preferences.set({ ...DEFAULT_PREFERENCES });
});

describe('the flat view', () => {
  it('Ctrl+B searches every file below the folder, lists the results when done, and Ctrl+B again goes back', async () => {
    const walk = walking();
    const backend = await appShown(invoke, walk.answers);

    await key(listbox('Left'), 'b', { ctrlKey: true });

    await waitFor(() => expect(walk.started).toHaveLength(1));
    expect(walk.started[0].args.query.root).toBe('/left');
    expect(walk.started[0].args.query.criteria).toMatchObject({ minSize: 0, skipHidden: true });
    expect(left().getByRole('status')).toHaveTextContent('Listing all files below this folder… (Ctrl+B or Esc stops)');

    walk.started[0].emit(done(2));
    await waitFor(() => expect(backend.sent('find_open')).toMatchObject([{ pane: 'left', id: 1 }]));
    await tick();

    expect(left().queryByRole('status')).not.toBeInTheDocument();
    expect(left().getByRole('navigation', { name: 'Path' })).toHaveTextContent('Flat view');
    expect(left().getByRole('option', { name: /^docs\/a txt/ })).toBeInTheDocument();

    await key(listbox('Left'), 'b', { ctrlKey: true });

    await waitFor(() => expect(backend.sent('pane_open').at(-1)).toMatchObject({ pane: 'left', path: '/left', child: null }));
  });

  it('skips hidden files unless the pane shows them', async () => {
    const walk = walking();
    await appShown(invoke, walk.answers);
    preferences.set({ showHidden: true });
    await tick();

    await key(listbox('Left'), 'b', { ctrlKey: true });

    await waitFor(() => expect(walk.started).toHaveLength(1));
    expect(walk.started[0].args.query.criteria).toMatchObject({ skipHidden: false });
  });

  it('is also run from the palette', async () => {
    const walk = walking();
    await appShown(invoke, walk.answers);

    await key(listbox('Left'), 'k', { ctrlKey: true });
    await fireEvent.input(screen.getByRole('combobox', { name: 'Command, folder or name' }), { target: { value: 'flat view' } });
    await fireEvent.keyDown(screen.getByRole('combobox', { name: 'Command, folder or name' }), { key: 'Enter' });

    await waitFor(() => expect(walk.started).toHaveLength(1));
  });

  it('Esc stops the walk, and what was found is listed with a notice that says so', async () => {
    const walk = walking();
    const backend = await appShown(invoke, walk.answers);
    await key(listbox('Left'), 'b', { ctrlKey: true });
    await waitFor(() => expect(walk.started).toHaveLength(1));

    expect(await key(listbox('Left'), 'Escape')).toBe(false); // handled: the default is prevented
    await waitFor(() => expect(backend.sent('find_cancel')).toEqual([{ id: 1 }]));
    walk.started[0].emit(done(2, { cancelled: true }));

    expect(await screen.findByText('Stopped: the 2 files found so far are listed.')).toBeInTheDocument();
    expect(backend.sent('find_open')).toHaveLength(1);
  });

  it('says when only the first files of a huge tree are listed', async () => {
    const walk = walking();
    await appShown(invoke, walk.answers);
    await key(listbox('Left'), 'b', { ctrlKey: true });
    await waitFor(() => expect(walk.started).toHaveLength(1));

    walk.started[0].emit(done(100000, { truncated: true }));

    expect(await screen.findByText('Only the first 100 000 files are listed.')).toBeInTheDocument();
  });

  it('forgets its search when the pane opens another folder', async () => {
    const walk = walking();
    const backend = await appShown(invoke, walk.answers);
    await key(listbox('Left'), 'b', { ctrlKey: true });
    await waitFor(() => expect(walk.started).toHaveLength(1));
    walk.started[0].emit(done(2));
    await waitFor(() => expect(backend.sent('find_open')).toHaveLength(1));

    await key(listbox('Left'), 'Backspace'); // up, out of the results
    await waitFor(() => expect(backend.sent('pane_open').at(-1)).toMatchObject({ pane: 'left', path: '/' }));
    backend.list('left', '/up', [entry('left', 'dir')]);

    await waitFor(() => expect(backend.sent('find_forget')).toEqual([{ id: 1 }]));
  });

  it('is refused inside an archive, and says why', async () => {
    const walk = walking();
    await withArchiveOnRight(invoke, 'd.zip', walk.answers);
    await key(document.body, 'Tab');

    await key(listbox('Right'), 'b', { ctrlKey: true });

    expect(await screen.findByText('A flat view lists a folder on disk: leave the archive first.')).toBeInTheDocument();
    expect(walk.started).toEqual([]);
  });
});
