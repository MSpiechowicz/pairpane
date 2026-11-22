import { fireEvent, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));
vi.mock('$lib/editor/cmMerge', () => import('../tests/diff'));

import type { MenuCommand } from '$lib/ipc/bindings/MenuCommand';

import { type Answers, appShown, key, listbox } from '../tests/app';
import { diffAnswers, merges } from '../tests/diff';
import { FakeFile, viewAnswers } from '../tests/view';

const shown = (answers: Answers = {}, documents = {}) => appShown(invoke, diffAnswers(documents, answers));

const overlay = () => screen.queryByRole('dialog', { name: 'Compare a.pdf and b.txt' });
const left = () => screen.getByRole('textbox', { name: 'Text of a.pdf' });
const right = () => screen.getByRole('textbox', { name: 'Text of b.txt' });
const notice = () => screen.findByRole('region', { name: 'Notice' });

/** Selects `a.pdf` and `b.txt` of the left pane (the cursor starts on `a.pdf`). */
async function selectTwo() {
  await key(listbox('Left'), 'Insert');
  await key(listbox('Left'), 'Insert');
}

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
  vi.restoreAllMocks();
  merges.length = 0;
});

describe('Shift+F2', () => {
  it('compares the two selected files of the active pane in a diff of the whole window', async () => {
    const backend = await shown();
    await selectTwo();

    await key(listbox('Left'), 'F2', { shiftKey: true });

    expect(await screen.findByRole('dialog', { name: 'Compare a.pdf and b.txt' })).toBeInTheDocument();
    await screen.findByRole('textbox', { name: 'Text of a.pdf' });
    expect(left()).toHaveValue('text of a.pdf\n');
    expect(right()).toHaveValue('text of b.txt\n');
    expect(backend.sent('edit_open')).toEqual([
      { path: '/left', name: 'a.pdf', encoding: null },
      { path: '/left', name: 'b.txt', encoding: null },
    ]);
    expect(screen.getByRole('toolbar', { name: 'Compare files' })).toBeInTheDocument();
  });

  it('compares the cursor file of each pane when nothing is selected', async () => {
    const backend = await shown();
    await key(document.body, 'Tab');
    await key(listbox('Right'), 'ArrowDown'); // c.md
    await key(document.body, 'Tab');

    await key(listbox('Left'), 'F2', { shiftKey: true });

    expect(await screen.findByRole('dialog', { name: 'Compare a.pdf and c.md' })).toBeInTheDocument();
    expect(backend.sent('edit_open')).toEqual([
      { path: '/left', name: 'a.pdf', encoding: null },
      { path: '/right', name: 'c.md', encoding: null },
    ]);
  });

  it('says why when the panes give no two files, and opens nothing', async () => {
    const backend = await shown();

    await key(listbox('Left'), 'F2', { shiftKey: true }); // the right cursor is on `..`

    expect(await notice()).toHaveTextContent('Folders cannot be compared by content.');
    expect(backend.sent('edit_open')).toHaveLength(0);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('plain F2 still renames in place', async () => {
    await shown();

    await key(listbox('Left'), 'F2');

    expect(screen.getByRole('textbox', { name: 'New name' })).toBeInTheDocument();
  });
});

describe('Files, Compare files…', () => {
  it('is in the menu with its key, and does the same as the key', async () => {
    const backend = await shown();
    await selectTwo();

    await fireEvent.click(screen.getByRole('button', { name: 'Files' }));
    const item = screen.getByRole('menuitem', { name: 'Compare files…' });
    expect(item).toHaveAttribute('aria-keyshortcuts', 'Shift+F2');
    await fireEvent.click(item);

    expect(await screen.findByRole('dialog', { name: 'Compare a.pdf and b.txt' })).toBeInTheDocument();
    expect(backend.sent('edit_open')).toHaveLength(2);
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });
});

describe('the text diff', () => {
  const open = async (extra: Answers = {}, documents = {}) => {
    const backend = await shown(extra, documents);
    await selectTwo();
    await key(listbox('Left'), 'F2', { shiftKey: true });
    await screen.findByRole('dialog', { name: 'Compare a.pdf and b.txt' });
    await screen.findByRole('textbox', { name: 'Text of b.txt' });

    return backend;
  };

  it('shows each side`s facts, the number of changes, and the shortcuts are the diff`s while it is open', async () => {
    const backend = await open();

    expect(within(screen.getByRole('group', { name: 'Left file' })).getByText('a.pdf')).toBeInTheDocument();
    expect(within(screen.getByRole('group', { name: 'Right file' })).getByText(/UTF-8 · LF/)).toBeInTheDocument();
    expect(screen.getByText('1 change')).toBeInTheDocument();

    await key(window, 'F5');
    await key(window, 'F7');

    expect(backend.sent('ops_start')).toHaveLength(0);
    expect(screen.queryByRole('dialog', { name: /New folder/ })).not.toBeInTheDocument();
  });

  it('F7 and Shift+F7 and the buttons move between the changes', async () => {
    await open();

    await key(window, 'F7');
    await key(window, 'F7', { shiftKey: true });
    await fireEvent.click(screen.getByRole('button', { name: 'Next difference' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Previous difference' }));

    expect(merges.at(-1)?.calls).toEqual(['expand', 'focus left', 'next', 'previous', 'next', 'previous']);
  });

  it('the Collapse unchanged toggle folds and unfolds', async () => {
    await open();

    const toggle = screen.getByRole('button', { name: 'Collapse unchanged' });
    await fireEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-pressed', 'true');
    await fireEvent.click(toggle);

    expect(toggle).toHaveAttribute('aria-pressed', 'false');
    expect(merges.at(-1)?.calls.slice(-2)).toEqual(['collapse', 'expand']);
  });

  it('Ctrl+S saves the side that has the focus, against its own revision, and only once', async () => {
    const backend = await open();
    await fireEvent.input(right(), { target: { value: 'changed on the right\n' } });
    expect(within(screen.getByRole('group', { name: 'Right file' })).getByRole('img', { name: 'Unsaved changes' })).toBeInTheDocument();

    right().focus();
    await key(right(), 's', { ctrlKey: true });

    await vi.waitFor(() => expect(backend.sent('edit_save')).toHaveLength(1));
    expect(backend.sent('edit_save')).toEqual([
      expect.objectContaining({ name: 'b.txt', text: 'changed on the right\n', options: expect.objectContaining({ revision: 'r-b.txt' }) }),
    ]);
    await vi.waitFor(() => expect(screen.queryByRole('img', { name: 'Unsaved changes' })).not.toBeInTheDocument());
    expect(overlay()).toBeInTheDocument();
  });

  it('a Save button writes its own side; a read-only side cannot be saved', async () => {
    const backend = await open({}, { 'b.txt': { readOnly: true } });
    await fireEvent.input(left(), { target: { value: 'changed on the left\n' } });

    const [leftSave, rightSave] = screen.getAllByRole('button', { name: 'Save' });
    expect(rightSave).toBeDisabled();
    await fireEvent.click(leftSave);

    await vi.waitFor(() => expect(backend.sent('edit_save')).toHaveLength(1));
    expect(backend.sent('edit_save')[0]).toMatchObject({ name: 'a.pdf' });
    expect(screen.getByText('This file is read-only: it cannot be changed or saved here.')).toBeInTheDocument();
  });

  it('a file that changed on disk is asked about, as in the editor', async () => {
    const answers = [{ kind: 'changed', gone: false }, { kind: 'saved', size: 1, revision: 'next' }];
    const backend = await open({ edit_save: () => answers.shift() });
    await fireEvent.input(left(), { target: { value: 'changed\n' } });

    left().focus();
    await key(left(), 's', { ctrlKey: true });
    const question = await screen.findByRole('dialog', { name: 'The file changed on disk' });
    await fireEvent.click(within(question).getByRole('button', { name: 'Overwrite' }));

    await vi.waitFor(() => expect(backend.sent('edit_save')).toHaveLength(2));
    expect(backend.sent('edit_save').map((args) => (args as { options: { force: boolean } }).options.force)).toEqual([false, true]);
  });

  it('Esc closes a clean diff; with changes it asks about both files first', async () => {
    const backend = await open();

    await key(document.body, 'Escape');
    await vi.waitFor(() => expect(overlay()).not.toBeInTheDocument());
    expect(backend.sent('editor_set_dirty')).toEqual([]);

    await key(listbox('Left'), 'F2', { shiftKey: true });
    await screen.findByRole('dialog', { name: 'Compare a.pdf and b.txt' });
    await screen.findByRole('textbox', { name: 'Text of a.pdf' });
    await fireEvent.input(left(), { target: { value: 'one\n' } });
    await fireEvent.input(right(), { target: { value: 'two\n' } });
    await key(document.body, 'Escape');

    const ask = await screen.findByRole('dialog', { name: 'Save changes to both files?' });
    await fireEvent.click(within(ask).getByRole('button', { name: 'Save' }));

    await vi.waitFor(() => expect(backend.sent('edit_save')).toHaveLength(2));
    await vi.waitFor(() => expect(overlay()).not.toBeInTheDocument());
    expect(backend.sent('editor_set_dirty')).toEqual([{ dirty: true }, { dirty: false }]);
  });
});

describe('leaving with changes', () => {
  const windowWorld = async () => {
    let send: (command: MenuCommand) => void = () => {};
    const backend = await shown({ menu_subscribe: (args: { onCommand: typeof send }) => void (send = args.onCommand) });
    await vi.waitFor(() => expect(backend.sent('menu_subscribe')).toHaveLength(1));
    await selectTwo();
    await key(listbox('Left'), 'F2', { shiftKey: true });
    await screen.findByRole('dialog', { name: 'Compare a.pdf and b.txt' });
    await screen.findByRole('textbox', { name: 'Text of a.pdf' });

    return { backend, send: (command: MenuCommand) => send(command) };
  };

  it('a window close with changes in the diff asks, and Don`t save lets it go on', async () => {
    const { backend, send } = await windowWorld();
    await fireEvent.input(right(), { target: { value: 'changed\n' } });

    send('closeRequested');
    const ask = await screen.findByRole('dialog', { name: 'Save changes to b.txt?' });
    await fireEvent.click(within(ask).getByRole('button', { name: "Don't save" }));

    await vi.waitFor(() => expect(backend.sent('app_close_confirmed')).toHaveLength(1));
    expect(backend.sent('edit_save')).toHaveLength(0);
    expect(backend.sent('editor_close_ack')).toHaveLength(1);
  });

  it('Cancel keeps the diff, and the backend hears again that there are changes', async () => {
    const { backend, send } = await windowWorld();
    await fireEvent.input(left(), { target: { value: 'changed\n' } });

    send('closeRequested');
    await screen.findByRole('dialog', { name: 'Save changes to a.pdf?' });
    const reported = backend.sent('editor_set_dirty').length;
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(backend.sent('app_close_confirmed')).toHaveLength(0);
    expect(left()).toHaveValue('changed\n');
    await vi.waitFor(() => expect(backend.sent('editor_set_dirty').slice(reported)).toEqual([{ dirty: true }]));
  });

  it('a window close with a clean diff goes on without asking', async () => {
    const { backend, send } = await windowWorld();

    send('closeRequested');

    await vi.waitFor(() => expect(backend.sent('app_close_confirmed')).toHaveLength(1));
    expect(screen.queryByRole('dialog', { name: /^Save changes/ })).not.toBeInTheDocument();
  });
});

describe('the byte diff', () => {
  const FILE = new FakeFile(new Uint8Array(100).map((_, index) => index));

  const open = async () => {
    const backend = await shown({
      ...viewAnswers(FILE, () => ({ kind: 'binary' })),
      edit_open: (args: { name: string }) => (args.name === 'b.txt' ? { kind: 'binary' } : diffAnswers().edit_open?.(args as never)),
      diff_next_difference: () => ({ kind: 'found', offset: 0x2b }),
    });
    await selectTwo();
    await key(listbox('Left'), 'F2', { shiftKey: true });

    return backend;
  };

  it('shows both files as bytes when one is not text, with the size line and the moves', async () => {
    const backend = await open();

    expect(await screen.findByRole('dialog', { name: 'Compare a.pdf and b.txt' })).toBeInTheDocument();
    expect(backend.sent('view_open')).toHaveLength(2);
    expect(await screen.findByText('100 bytes / 100 bytes, first difference at 0x2B')).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Bytes of a.pdf and b.txt' })).toBeInTheDocument();
    expect(screen.queryByRole('textbox', { name: /Text of/ })).not.toBeInTheDocument();

    await key(window, 'F7');
    await vi.waitFor(() => expect(backend.sent('diff_next_difference')).toHaveLength(2));
    expect(backend.sent('diff_next_difference').at(-1)).toEqual({ left: 1, right: 2, from: 0, forward: true });
  });

  it('Esc closes it at once and lets both sessions go', async () => {
    const backend = await open();
    await screen.findByRole('textbox', { name: 'Bytes of a.pdf and b.txt' });

    await key(document.body, 'Escape');

    await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(backend.sent('view_close')).toEqual([{ session: 1 }, { session: 2 }]);
    await tick();
  });
});
