import { fireEvent, screen, waitFor, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import type { FindEvent } from '$lib/ipc/bindings/FindEvent';
import type { ListEvent } from '$lib/ipc/bindings/ListEvent';
import type { SavedState } from '$lib/ipc/bindings/SavedState';

import { type Answers, appShown, key, listbox, withArchiveOnRight } from '../tests/app';
import { done, hit } from '../tests/find';

/** The search the page started: its channel, for the test to send events on. */
function searching() {
  const started: { args: { query: { root: string } }; emit: (event: FindEvent) => void }[] = [];
  const answers: Answers = {
    find_start: (args: { onEvent: (event: FindEvent) => void; query: { root: string } }) => {
      started.push({ args, emit: args.onEvent });
      return started.length;
    },
    // The backend lists the results into the pane before it answers: the pane shows them from the `start` on.
    find_open: (args: { id: number; onEvent: (event: ListEvent) => void }) => {
      args.onEvent({ kind: 'start', path: '/left', parent: '/', crumbs: [], total: 0, results: args.id });
      args.onEvent({ kind: 'done', dirs: 0, files: 0, bytes: 0, free: null });
      return null;
    },
  };
  return { started, answers };
}

const dialog = () => screen.getByRole('dialog', { name: 'Find files' });
const root = () => within(dialog()).getByText(/^\//).textContent;

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
});

describe('opening Find', () => {
  it('Alt+F7 opens it on the folder of the active pane', async () => {
    await appShown(invoke);

    await key(listbox('Left'), 'F7', { altKey: true });

    expect(root()).toBe('/left');
    expect(screen.getByRole('textbox', { name: 'Names' })).toHaveFocus();
  });

  it('the other pane, when it is the active one, is the folder', async () => {
    await appShown(invoke);
    await key(document.body, 'Tab');

    await key(listbox('Right'), 'F7', { altKey: true });

    expect(root()).toBe('/right');
  });

  it('Files, Find files… and the search button of the top bar open it too', async () => {
    await appShown(invoke);

    await fireEvent.click(screen.getByRole('button', { name: 'Files' }));
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Find files…' }));
    expect(dialog()).toBeInTheDocument();
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: /Find files…/ }));
    expect(dialog()).toBeInTheDocument();
  });

  it('inside an archive it looks in the folder that holds the archive', async () => {
    await withArchiveOnRight(invoke, 'd.zip');
    await key(document.body, 'Tab');

    await key(listbox('Right'), 'F7', { altKey: true });

    expect(root()).toBe('/right');
  });

  it('has the keyboard while it is open: the panes get none of it, and Escape closes it', async () => {
    const backend = await appShown(invoke);
    await key(listbox('Left'), 'F7', { altKey: true });

    await key(window, 'F7');
    await key(window, 'Tab');
    expect(screen.getByRole('textbox', { name: 'Shell command' })).toBeInTheDocument();
    expect(backend.sent('ops_mkdir')).toEqual([]);
    expect(screen.queryByRole('dialog', { name: 'New folder' })).not.toBeInTheDocument();

    await key(window, 'Escape');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});

describe('results', () => {
  it('Go to opens the folder of the result in the active pane with the cursor on it, and closes the dialog', async () => {
    const find = searching();
    const backend = await appShown(invoke, find.answers);
    await key(listbox('Left'), 'F7', { altKey: true });
    await fireEvent.click(screen.getByRole('button', { name: 'Start' }));
    await waitFor(() => expect(find.started).toHaveLength(1));
    expect(find.started[0].args.query.root).toBe('/left');
    find.started[0].emit({ kind: 'found', hits: [hit('sub/deeper/c.md')] });
    find.started[0].emit(done(1));
    await tick();

    await fireEvent.click(screen.getByRole('button', { name: 'Go to' }));

    await waitFor(() => expect(backend.sent('pane_open').at(-1)).toMatchObject({ pane: 'left', path: '/left/sub/deeper' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    await waitFor(() => expect(backend.sent('find_forget')).toEqual([{ id: 1 }]));
  });

  it('a result inside an archive opens the archive at the folder of the entry', async () => {
    const find = searching();
    const backend = await appShown(invoke, find.answers);
    await key(listbox('Left'), 'F7', { altKey: true });
    await fireEvent.click(screen.getByRole('button', { name: 'Start' }));
    await waitFor(() => expect(find.started).toHaveLength(1));
    find.started[0].emit({ kind: 'found', hits: [hit('backups/old.zip', { inner: 'docs/a.txt' })] });
    find.started[0].emit(done(1));
    await tick();

    await fireEvent.click(screen.getByRole('button', { name: 'Go to' }));

    await waitFor(() =>
      expect(backend.sent('archive_open').at(-1)).toMatchObject({ pane: 'left', archive: '/left/backups/old.zip', dir: 'docs' }),
    );
  });

  it('Feed to panel lists the results in the active pane, which keeps them when the dialog is gone', async () => {
    const find = searching();
    const backend = await appShown(invoke, find.answers);
    await key(listbox('Left'), 'F7', { altKey: true });
    await fireEvent.click(screen.getByRole('button', { name: 'Start' }));
    await waitFor(() => expect(find.started).toHaveLength(1));
    find.started[0].emit({ kind: 'found', hits: [hit('a/b.txt')] });
    find.started[0].emit(done(1));
    await tick();

    await fireEvent.click(screen.getByRole('button', { name: 'Feed to panel' }));

    await waitFor(() => expect(backend.sent('find_open')).toEqual([expect.objectContaining({ pane: 'left', id: 1 })]));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(backend.sent('find_forget')).toEqual([]);
  });
});

describe('saved searches', () => {
  const state: SavedState = {
    active: 'left',
    left: { tabs: [{ path: '/left', sort: 'name', desc: false }], active: 0 },
    right: { tabs: [{ path: '/right', sort: 'name', desc: false }], active: 0 },
    hotlist: [],
  };

  it('come back from the state file and every save carries them, a deleted one gone', async () => {
    const criteria = {
      names: '*.log', nameRegex: false, namePath: false, nameCase: false, text: null, minSize: null, maxSize: null,
      modifiedFrom: null, modifiedTo: null, hidden: 'any', readonly: 'any', executable: 'any', skipHidden: false,
      skipIgnored: false, oneFilesystem: false, archives: false, duplicates: false,
    } as const;
    const backend = await appShown(invoke, { state_load: () => ({ ...state, searches: [{ name: 'Logs', criteria }] }) });
    await key(listbox('Left'), 'F7', { altKey: true });
    const picker = screen.getByRole('combobox', { name: 'Saved search' });
    expect(within(picker).getByRole('option', { name: 'Logs' })).toBeInTheDocument();

    await fireEvent.change(picker, { target: { value: 'Logs' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Load' }));
    expect(screen.getByRole('textbox', { name: 'Names' })).toHaveValue('*.log');

    await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));

    await waitFor(() => expect((backend.sent('state_save').at(-1) as { state: SavedState }).state.searches).toEqual([]));
  });

  it('a search saved from the dialog is written with the state', async () => {
    const backend = await appShown(invoke, { state_load: () => state });
    await key(listbox('Left'), 'F7', { altKey: true });
    await fireEvent.input(screen.getByRole('textbox', { name: 'Names' }), { target: { value: '*.rs' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Save as…' }));
    const name = screen.getByRole('textbox', { name: 'Name of the search' });
    await fireEvent.input(name, { target: { value: 'Rust' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() => {
      const saves = backend.sent('state_save') as { state: SavedState }[];
      expect(saves.at(-1)?.state.searches).toEqual([{ name: 'Rust', criteria: expect.objectContaining({ names: '*.rs' }) }]);
    });
  });
});
