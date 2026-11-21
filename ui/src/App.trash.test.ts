import { fireEvent, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import { IpcFailure } from '$lib/ipc/transport';

import { type Answers, appShown, key, listbox, pane } from '../tests/app';
import { entry } from '../tests/panes';

const shown = (answers: Answers = {}) => appShown(invoke, answers);
const refuse = (code: 'invalidInput' | 'process' | 'notFound', message: string) => async () => {
  throw new IpcFailure({ code, message });
};
const left = () => within(pane('Left pane'));

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
});

describe('moving to the Trash', () => {
  it('F8 asks first, with the confirm button focused; confirming moves the item and offers Undo', async () => {
    const backend = await shown({ ops_trash: () => ({ undo: 4, failure: null }), ops_untrash: () => 1 });

    expect(await key(listbox('Left'), 'F8')).toBe(false);
    const confirm = screen.getByRole('button', { name: 'Move to Trash' });
    expect(screen.getByRole('dialog', { name: 'Move “a.pdf” to the Trash?' })).toBeInTheDocument();
    expect(confirm).toHaveFocus();
    expect(backend.sent('ops_trash')).toEqual([]);

    await fireEvent.click(confirm);

    await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(backend.sent('ops_trash')).toEqual([{ path: '/left', names: ['a.pdf'] }]);
    expect(screen.getByRole('region', { name: 'Notice' })).toHaveTextContent('Moved “a.pdf” to the Trash.');
    expect(listbox('Left')).toHaveFocus();

    await fireEvent.click(screen.getByRole('button', { name: 'Undo' }));

    expect(backend.sent('ops_untrash')).toEqual([{ token: 4 }]);
    expect(await screen.findByText('Restored 1 item.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Undo' })).not.toBeInTheDocument();
  });

  it('the notice is closed with its button, and the list has the focus again', async () => {
    await shown({ ops_trash: () => ({ undo: 4, failure: null }) });
    await key(listbox('Left'), 'F8');
    await fireEvent.click(screen.getByRole('button', { name: 'Move to Trash' }));
    const close = await screen.findByRole('button', { name: 'Close' });

    close.focus();
    await fireEvent.click(close);

    expect(screen.queryByRole('region', { name: 'Notice' })).not.toBeInTheDocument();
    await vi.waitFor(() => expect(listbox('Left')).toHaveFocus());
  });

  it('there is no Undo without a token (macOS)', async () => {
    await shown({ ops_trash: () => ({ undo: null, failure: null }) });

    await key(listbox('Left'), 'F8');
    await fireEvent.click(screen.getByRole('button', { name: 'Move to Trash' }));

    expect(await screen.findByText('Moved “a.pdf” to the Trash.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Undo' })).not.toBeInTheDocument();
  });

  it('Undo is clicked once: a second click has nothing to undo', async () => {
    let restored: (count: number) => void = () => {};
    const backend = await shown({
      ops_trash: () => ({ undo: 4, failure: null }),
      ops_untrash: () => new Promise((resolve) => (restored = resolve)),
    });
    await key(listbox('Left'), 'Delete');
    await fireEvent.click(screen.getByRole('button', { name: 'Move to Trash' }));
    const undo = await screen.findByRole('button', { name: 'Undo' });

    await fireEvent.click(undo);
    expect(screen.queryByRole('button', { name: 'Undo' })).not.toBeInTheDocument();
    restored(2);

    expect(await screen.findByText('Restored 2 items.')).toBeInTheDocument();
    expect(backend.sent('ops_untrash')).toHaveLength(1);
  });

  it('a stale Undo says why it failed', async () => {
    await shown({
      ops_trash: () => ({ undo: 4, failure: null }),
      ops_untrash: refuse('notFound', 'There is nothing to undo.'),
    });
    await key(listbox('Left'), 'F8');
    await fireEvent.click(screen.getByRole('button', { name: 'Move to Trash' }));

    await fireEvent.click(await screen.findByRole('button', { name: 'Undo' }));

    expect(await screen.findByText('There is nothing to undo.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Undo' })).not.toBeInTheDocument();
  });

  it('an Undo blocked by an item in the way can be tried again; a stale one cannot', async () => {
    await shown({
      ops_trash: () => ({ undo: 4, failure: null }),
      ops_untrash: refuse('invalidInput', 'Cannot restore: an item named a.pdf already exists where it was.'),
    });
    await key(listbox('Left'), 'F8');
    await fireEvent.click(screen.getByRole('button', { name: 'Move to Trash' }));
    await fireEvent.click(await screen.findByRole('button', { name: 'Undo' }));

    expect(await screen.findByText(/already exists where it was/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Undo' })).toBeInTheDocument();
  });

  it('a move that stopped part way says so and still offers Undo for what went', async () => {
    await shown({
      ops_trash: () => ({ undo: 9, failure: { code: 'io', message: 'Could not move that item to the Trash.' } }),
    });
    await key(listbox('Left'), 'Insert');
    await key(listbox('Left'), 'Insert');

    await key(listbox('Left'), 'F8');
    expect(screen.getByRole('dialog', { name: 'Move 2 items to the Trash?' })).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Move to Trash' }));

    const notice = await screen.findByRole('region', { name: 'Notice' });
    expect(notice).toHaveTextContent('Could not move that item to the Trash.');
    expect(within(notice).getByRole('button', { name: 'Undo' })).toBeInTheDocument();
    expect(left().getByText(/^Selected:/)).toBeInTheDocument();
  });

  it('a refusal is shown in the dialog, which stays open', async () => {
    await shown({ ops_trash: refuse('invalidInput', 'That folder is not allowed.') });
    await key(listbox('Left'), 'F8');

    await fireEvent.click(screen.getByRole('button', { name: 'Move to Trash' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('That folder is not allowed.');
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Move to Trash' })).toBeEnabled();
  });

  it('Escape and Cancel move nothing', async () => {
    const backend = await shown();

    await key(listbox('Left'), 'F8');
    await key(screen.getByRole('button', { name: 'Move to Trash' }), 'Escape');
    await key(listbox('Left'), 'F8');
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(backend.sent('ops_trash')).toEqual([]);
    expect(listbox('Left')).toHaveFocus();
  });

  it('takes the selection, and a done move clears it', async () => {
    const backend = await shown({ ops_trash: () => ({ undo: 1, failure: null }) });
    await key(listbox('Left'), 'Insert');
    await key(listbox('Left'), 'Insert');
    expect(left().getByText(/^Selected:/)).toBeInTheDocument();

    await key(listbox('Left'), 'Delete');
    await fireEvent.click(screen.getByRole('button', { name: 'Move to Trash' }));

    await vi.waitFor(() => expect(left().queryByText(/^Selected:/)).not.toBeInTheDocument());
    expect(backend.sent('ops_trash')).toEqual([{ path: '/left', names: ['a.pdf', 'b.txt'] }]);
  });

  it('the `..` row and an empty folder are no items', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'Home');

    await key(listbox('Left'), 'F8');
    await key(listbox('Left'), 'F8', { shiftKey: true });

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(backend.sent('ops_trash')).toEqual([]);
  });

  it('acts only on the selected items the quick filter shows', async () => {
    const backend = await shown({ ops_trash: () => ({ undo: 1, failure: null }) });
    await key(listbox('Left'), 'a', { ctrlKey: true });
    await key(listbox('Left'), 't'); // only b.txt matches

    await key(listbox('Left'), 'F8');
    expect(screen.getByRole('dialog', { name: 'Move “b.txt” to the Trash?' })).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Move to Trash' }));

    await vi.waitFor(() => expect(backend.sent('ops_trash')).toHaveLength(1));
    expect(backend.sent('ops_trash')[0]).toEqual({ path: '/left', names: ['b.txt'] });
  });

  it('the dock`s F8 button asks the same', async () => {
    await shown();

    await fireEvent.click(screen.getByRole('button', { name: 'Delete (F8)' }));

    expect(screen.getByRole('dialog', { name: 'Move “a.pdf” to the Trash?' })).toBeInTheDocument();
  });

  it('says Recycle Bin on Windows', async () => {
    vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Mozilla/5.0 (Windows NT 10.0; Win64; x64)');
    await shown();

    await key(listbox('Left'), 'F8');

    expect(screen.getByRole('dialog', { name: 'Move “a.pdf” to the Recycle Bin?' })).toBeInTheDocument();
    vi.restoreAllMocks();
  });
});

describe('deleting for good', () => {
  it('Shift+F8 warns, has the focus on Cancel and the danger button deletes', async () => {
    const backend = await shown();

    await key(listbox('Left'), 'F8', { shiftKey: true });
    expect(screen.getByRole('dialog', { name: 'Delete “a.pdf” permanently?' })).toBeInTheDocument();
    expect(screen.getByText('This cannot be undone.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus();
    expect(backend.sent('ops_delete')).toEqual([]);

    await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));

    await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(backend.sent('ops_delete')).toEqual([{ path: '/left', names: ['a.pdf'] }]);
    expect(backend.sent('ops_trash')).toEqual([]);
  });

  it('Shift+Delete does the same, and a plain Delete only moves to the Trash', async () => {
    await shown();

    await key(listbox('Left'), 'Delete', { shiftKey: true });
    expect(screen.getByRole('dialog', { name: 'Delete “a.pdf” permanently?' })).toBeInTheDocument();
    await key(screen.getByRole('button', { name: 'Cancel' }), 'Escape');

    await key(listbox('Left'), 'Delete');
    expect(screen.getByRole('dialog', { name: 'Move “a.pdf” to the Trash?' })).toBeInTheDocument();
  });

  it('a refusal is shown in the dialog', async () => {
    await shown({ ops_delete: refuse('notFound', 'That item is gone.') });
    await key(listbox('Left'), 'F8', { shiftKey: true });

    await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('That item is gone.');
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('Shift+Delete in an open drive menu is the menu`s, not a file`s', async () => {
    const backend = await shown();
    await key(document.body, 'F1', { altKey: true });
    const drives = await screen.findByRole('listbox', { name: 'Drives' });

    await key(drives, 'Delete', { shiftKey: true });
    await key(drives, 'Delete');
    await key(drives, 'F8');

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(backend.sent('ops_delete')).toEqual([]);
    expect(backend.sent('ops_trash')).toEqual([]);
  });
});
