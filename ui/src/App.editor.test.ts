import { fireEvent, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));
vi.mock('$lib/editor/cm', () => import('../tests/editor'));

import type { MenuCommand } from '$lib/ipc/bindings/MenuCommand';

import { type Answers, appShown, key, listbox } from '../tests/app';
import { editAnswers } from '../tests/editor';
import { FakeFile, numbered, viewAnswers } from '../tests/view';

const shown = (answers: Answers = {}, document = {}) => appShown(invoke, editAnswers(document, answers));

const overlay = () => screen.queryByRole('dialog', { name: 'a.pdf' });
const text = () => screen.getByRole('textbox', { name: 'Text of a.pdf' });
const refuse = (code: string, message: string) => () => Promise.reject({ code, message });
const type = (value: string) => fireEvent.input(text(), { target: { value } });

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
  vi.restoreAllMocks();
});

describe('F4 editor', () => {
  it('F4 tries the external editor, then opens the file under the cursor in the built-in one', async () => {
    const backend = await shown();

    await key(listbox('Left'), 'F4');

    expect(await screen.findByRole('textbox', { name: 'Text of a.pdf' })).toHaveValue('one\ntwo\n');
    expect(overlay()).toBeInTheDocument();
    expect(backend.sent('edit_external')).toEqual([{ path: '/left', name: 'a.pdf' }]);
    expect(backend.sent('edit_open')).toEqual([{ path: '/left', name: 'a.pdf', encoding: null }]);
  });

  it('the dock`s Edit (F4) button does the same as the key', async () => {
    const backend = await shown();

    await fireEvent.click(screen.getByRole('button', { name: 'Edit (F4)' }));

    expect(await screen.findByRole('textbox', { name: 'Text of a.pdf' })).toBeInTheDocument();
    expect(backend.sent('edit_open')).toHaveLength(1);
  });

  it('does nothing on a folder or on the parent row', async () => {
    const backend = await shown();

    await key(listbox('Left'), 'ArrowUp'); // `..`
    await key(listbox('Left'), 'F4');
    await key(listbox('Left'), 'ArrowDown');
    await key(listbox('Left'), 'ArrowDown');
    await key(listbox('Left'), 'ArrowDown'); // `sub`
    await key(listbox('Left'), 'F4');

    expect(backend.sent('edit_external')).toHaveLength(0);
    expect(backend.sent('edit_open')).toHaveLength(0);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('Shift+F4 is still a new file, not an edit', async () => {
    const backend = await shown();

    await key(listbox('Left'), 'F4', { shiftKey: true });

    expect(screen.getByRole('textbox', { name: 'File name' })).toBeInTheDocument();
    expect(backend.sent('edit_external')).toHaveLength(0);
  });

  it('with an external editor set up it is started and no overlay opens', async () => {
    const backend = await shown({ edit_external: () => 'launched' });

    await key(listbox('Left'), 'F4');

    await vi.waitFor(() => expect(backend.sent('edit_external')).toHaveLength(1));
    expect(backend.sent('edit_open')).toHaveLength(0);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('shows why the external editor could not be started, and opens nothing', async () => {
    const backend = await shown({ edit_external: refuse('process', 'the external editor could not be started') });

    await key(listbox('Left'), 'F4');

    expect(await screen.findByText('the external editor could not be started')).toBeInTheDocument();
    expect(backend.sent('edit_open')).toHaveLength(0);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('the Files menu`s built-in editor item skips the external one', async () => {
    const backend = await shown({ edit_external: () => 'launched' });

    await fireEvent.click(screen.getByRole('button', { name: 'Files' }));
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Edit in built-in editor' }));

    expect(await screen.findByRole('textbox', { name: 'Text of a.pdf' })).toBeInTheDocument();
    expect(backend.sent('edit_external')).toHaveLength(0);
  });

  it('a file too large to edit goes to the viewer, which says why', async () => {
    const view = viewAnswers(new FakeFile(numbered(10)), () => ({ kind: 'text' }));
    const backend = await appShown(invoke, { ...view, ...editAnswers({}, { edit_open: () => ({ kind: 'tooLarge', size: 9 * 1024 * 1024 }) }) });

    await key(listbox('Left'), 'F4');

    expect(await screen.findByRole('dialog', { name: 'a.pdf' })).toBeInTheDocument();
    expect(screen.getByText(/too large to edit/)).toBeInTheDocument();
    expect(backend.sent('view_open')).toEqual([{ path: '/left', name: 'a.pdf' }]);
  });

  it('a file that is not text is refused with a notice', async () => {
    await shown({ edit_open: () => ({ kind: 'binary' }) });

    await key(listbox('Left'), 'F4');

    expect(await screen.findByText(/not a text file; F3 views it/)).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('the pane shortcuts are off while it is open: F5 does not copy and Ctrl+T opens no tab', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'F4');
    await screen.findByRole('textbox', { name: 'Text of a.pdf' });
    const opened = backend.sent('pane_open').length;

    await key(window, 'F5');
    await key(window, 'F7');
    await key(window, 't', { ctrlKey: true });

    expect(screen.getAllByRole('dialog')).toHaveLength(1);
    expect(backend.sent('ops_start')).toHaveLength(0);
    expect(backend.sent('pane_open')).toHaveLength(opened);
  });

  it('closing it gives the list the focus back', async () => {
    await shown();
    await key(listbox('Left'), 'F4');
    await screen.findByRole('textbox', { name: 'Text of a.pdf' });

    await key(window, 'Escape');

    expect(overlay()).not.toBeInTheDocument();
    await vi.waitFor(() => expect(listbox('Left')).toHaveFocus());
  });
});

describe('Settings', () => {
  const choose = async () => {
    await fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Edit Settings…' }));
  };

  it('Edit Settings asks the backend to open the settings file', async () => {
    const backend = await shown();

    await choose();

    expect(backend.sent('settings_open')).toHaveLength(1);
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('is there on macOS too, without the update check the native menu has', async () => {
    vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)');
    const backend = await shown();

    await choose();

    expect(backend.sent('settings_open')).toHaveLength(1);
  });

  it('shows why the file could not be opened', async () => {
    await shown({ settings_open: refuse('process', 'the settings file could not be opened') });

    await choose();

    expect(await screen.findByText('the settings file could not be opened')).toBeInTheDocument();
  });
});

describe('leaving with changes', () => {
  /** The window; the test sends the native menu's commands. */
  const windowWorld = async (extra: Answers = {}) => {
    let send: (command: MenuCommand) => void = () => {};
    const backend = await shown({ menu_subscribe: (args: { onCommand: typeof send }) => void (send = args.onCommand), ...extra });
    await vi.waitFor(() => expect(backend.sent('menu_subscribe')).toHaveLength(1));

    return { backend, send: (command: MenuCommand) => send(command) };
  };

  const change = async () => {
    await key(listbox('Left'), 'F4');
    await screen.findByRole('textbox', { name: 'Text of a.pdf' });
    await type('changed');
  };

  const ask = () => screen.findByRole('dialog', { name: /^Save changes/ });

  it('a window close with changes asks, and Don`t save lets it go on', async () => {
    const { backend, send } = await windowWorld();
    await change();

    send('closeRequested');
    await fireEvent.click(await screen.findByRole('button', { name: "Don't save" }));

    await vi.waitFor(() => expect(backend.sent('app_close_confirmed')).toHaveLength(1));
    expect(backend.sent('edit_save')).toHaveLength(0);
  });

  it('Save writes first, and the window closes after it', async () => {
    const { backend, send } = await windowWorld();
    await change();

    send('closeRequested');
    const dialog = await ask();
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Save' }));

    await vi.waitFor(() => expect(backend.sent('app_close_confirmed')).toHaveLength(1));
    expect(backend.sent('edit_save')).toHaveLength(1);
  });

  it('the backend is told at once that the user is being asked, without waiting for an answer; a refused tell changes nothing', async () => {
    const { backend, send } = await windowWorld({ editor_close_ack: refuse('internal', 'no window') });
    await change();

    send('closeRequested');

    expect(backend.sent('editor_close_ack')).toHaveLength(1);
    expect(backend.sent('app_close_confirmed')).toHaveLength(0);
    await ask();
  });

  it('a second close request replaces the first question: only the one on screen acts', async () => {
    const { backend, send } = await windowWorld();
    await change();
    const reported = backend.sent('editor_set_dirty').length;

    send('closeRequested');
    await ask();
    send('closeRequested');
    await tick();
    await fireEvent.click(await screen.findByRole('button', { name: "Don't save" }));

    await vi.waitFor(() => expect(backend.sent('app_close_confirmed')).toHaveLength(1));
    expect(backend.sent('editor_set_dirty').slice(reported)).toEqual([]);
    expect(backend.sent('editor_close_ack')).toHaveLength(2);
  });

  it('Cancel keeps the window open', async () => {
    const { backend, send } = await windowWorld();
    await change();

    send('closeRequested');
    await ask();
    const reported = backend.sent('editor_set_dirty').length;
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(backend.sent('app_close_confirmed')).toHaveLength(0);
    expect(text()).toHaveValue('changed');
    // The backend is told again that the changes are there, which ends its pending close.
    await vi.waitFor(() => expect(backend.sent('editor_set_dirty').slice(reported)).toEqual([{ dirty: true }]));
  });

  it('a window close with nothing changed goes on without asking', async () => {
    const { backend, send } = await windowWorld();

    send('closeRequested');

    await vi.waitFor(() => expect(backend.sent('app_close_confirmed')).toHaveLength(1));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('a restart after an update asks first, and Cancel keeps the app running', async () => {
    const info = { version: '9.9.9', currentVersion: '0.1.0', date: null, notes: null };
    const { backend } = await windowWorld({ update_check: () => info });
    await fireEvent.click(await screen.findByRole('button', { name: 'Install' }));
    const restart = await screen.findByRole('button', { name: 'Restart' });
    await change();

    await fireEvent.click(restart);
    await ask();
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(backend.sent('app_relaunch')).toHaveLength(0);

    await fireEvent.click(restart);
    await ask();
    await fireEvent.click(screen.getByRole('button', { name: "Don't save" }));
    await vi.waitFor(() => expect(backend.sent('app_relaunch')).toHaveLength(1));
  });

  it('a restart with nothing changed goes on at once', async () => {
    const info = { version: '9.9.9', currentVersion: '0.1.0', date: null, notes: null };
    const { backend } = await windowWorld({ update_check: () => info });
    await fireEvent.click(await screen.findByRole('button', { name: 'Install' }));

    await fireEvent.click(await screen.findByRole('button', { name: 'Restart' }));

    await vi.waitFor(() => expect(backend.sent('app_relaunch')).toHaveLength(1));
  });
});
