import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import type { MenuCommand } from '$lib/ipc/bindings/MenuCommand';
import type { Preferences } from '$lib/ipc/bindings/Preferences';
import type { SavedState } from '$lib/ipc/bindings/SavedState';
import { IpcFailure } from '$lib/ipc/transport';
import { DEFAULT_PREFERENCES, preferences } from '$lib/shell/preferences.svelte';

import App from './App.svelte';
import { type Answers, appShown, appWorld, key, listbox, pane, withArchiveOnRight } from '../tests/app';
import { job } from '../tests/ops';
import { entry } from '../tests/panes';

const shown = (answers: Answers = {}) => appShown(invoke, answers);

/** A saved session (both panes in their folders) with these preferences. */
const savedWith = (changes: Partial<Preferences>): SavedState => ({
  active: 'left',
  left: { tabs: [{ path: '/left', sort: 'name', desc: false }], active: 0 },
  right: { tabs: [{ path: '/right', sort: 'name', desc: false }], active: 0 },
  hotlist: [],
  preferences: { ...DEFAULT_PREFERENCES, ...changes },
});

const settings = () => screen.getByRole('dialog', { name: 'Settings' });
const left = () => within(pane('Left pane'));

async function openFromMenu() {
  await fireEvent.click(screen.getByRole('button', { name: /^Settings/ }));
  await fireEvent.click(screen.getByRole('menuitem', { name: 'Settings…' }));
}

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
  delete document.documentElement.dataset.theme;
});

describe('opening the settings', () => {
  it('is the first item of the Settings menu', async () => {
    await shown();

    await openFromMenu();

    expect(settings()).toBeInTheDocument();
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('is a command of the palette', async () => {
    await shown();
    await key(listbox('Left'), 'k', { ctrlKey: true });

    const field = screen.getByRole('combobox', { name: 'Command, folder or name' });
    await fireEvent.input(field, { target: { value: 'settings…' } });
    await fireEvent.keyDown(field, { key: 'Enter' });

    expect(settings()).toBeInTheDocument();
  });

  it('leaves the keyboard to the dialog, and Escape gives it back to the list', async () => {
    const backend = await shown();
    await openFromMenu();

    await key(listbox('Left'), 'F7'); // a shortcut of the panes is not run beneath the dialog
    expect(screen.queryByRole('dialog', { name: 'New folder' })).not.toBeInTheDocument();
    expect(backend.sent('ops_mkdir')).toEqual([]);
    await key(settings(), 'Escape');

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    await vi.waitFor(() => expect(listbox('Left')).toHaveFocus());
  });

  it('opens the settings file from its button, and closes', async () => {
    const backend = await shown();
    await openFromMenu();

    await fireEvent.click(within(settings()).getByRole('button', { name: 'Edit settings file…' }));

    expect(backend.sent('settings_open')).toHaveLength(1);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});

describe('what a change does at once', () => {
  it('the Size and Modified boxes take the columns out of the header and the rows', async () => {
    await shown();
    await openFromMenu();
    expect(left().getByRole('button', { name: 'Size' })).toBeInTheDocument();

    await fireEvent.click(within(settings()).getByRole('checkbox', { name: 'Size' }));
    await fireEvent.click(within(settings()).getByRole('checkbox', { name: 'Modified' }));

    expect(left().queryByRole('button', { name: 'Size' })).not.toBeInTheDocument();
    expect(left().queryByRole('button', { name: 'Modified' })).not.toBeInTheDocument();
    expect(left().getByRole('button', { name: 'Ext' })).toBeInTheDocument();
    expect(document.querySelector('.row .sz')).toBeNull();
    expect(document.querySelector('.row .dt')).toBeNull();
    expect(document.querySelector('.row .ex')).not.toBeNull();
  });

  it('the hidden-files box and the Hidden button of the top bar are one setting', async () => {
    const backend = await shown();
    const button = screen.getByRole('button', { name: 'Hidden' });
    expect(button).toHaveAttribute('aria-pressed', 'false');
    await openFromMenu();

    await fireEvent.click(within(settings()).getByRole('checkbox', { name: 'Show hidden files' }));

    expect(button).toHaveAttribute('aria-pressed', 'true');
    await vi.waitFor(() => expect(backend.sent('pane_open').slice(-2)).toMatchObject([{ showHidden: true }, { showHidden: true }]));
    await fireEvent.click(screen.getByRole('button', { name: 'Close' }));

    await fireEvent.click(button);

    expect(button).toHaveAttribute('aria-pressed', 'false');
    expect(preferences.value.showHidden).toBe(false);
    await vi.waitFor(() => expect(backend.sent('pane_open').slice(-2)).toMatchObject([{ showHidden: false }, { showHidden: false }]));
  });

  it('the theme radio buttons and the theme button of the top bar are one setting', async () => {
    await shown();
    await openFromMenu();

    await fireEvent.click(within(settings()).getByRole('radio', { name: 'Dark' }));
    await vi.waitFor(() => expect(document.documentElement.dataset.theme).toBe('dark'));
    expect(window.localStorage.getItem('pairpane.theme')).toBe('dark');
    await fireEvent.click(screen.getByRole('button', { name: 'Close' }));

    await fireEvent.click(screen.getByRole('button', { name: 'Theme: Dark' })); // Dark → System
    expect(preferences.value.theme).toBe('system');
    await fireEvent.click(screen.getByRole('button', { name: 'Theme: System' }));
    expect(preferences.value.theme).toBe('light');
    expect(document.documentElement.dataset.theme).toBe('light');
  });

  it('every change is saved with the session', async () => {
    const backend = await shown();
    await openFromMenu();

    await fireEvent.click(within(settings()).getByRole('checkbox', { name: 'Ask before moving to the Trash' }));
    await fireEvent.click(within(settings()).getByRole('radio', { name: 'Light' }));

    await vi.waitFor(() =>
      expect(backend.sent('state_save').at(-1)).toMatchObject({
        state: { preferences: { ...DEFAULT_PREFERENCES, confirmTrash: false, theme: 'light' } },
      }),
    );
  });
});

describe('what is restored', () => {
  it('the columns, the hidden files, the questions and the theme of the last session', async () => {
    const backend = await shown({
      state_load: () => savedWith({ theme: 'dark', showExt: false, showHidden: true, confirmTrash: false }),
    });

    expect(left().queryByRole('button', { name: 'Ext' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Hidden' })).toHaveAttribute('aria-pressed', 'true');
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(backend.sent('pane_open').map((open) => (open as { showHidden: boolean }).showHidden)).toEqual([true, true]);
    expect(backend.sent('state_save')).toEqual([]); // what was loaded is not a change
  });

  it('the page shows the last theme before the session is there, and the session then has the say', async () => {
    window.localStorage.setItem('pairpane.theme', 'light');
    let load: (state: SavedState) => void = () => {};
    const world = shown({ state_load: () => new Promise((resolve) => (load = resolve)) });
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('state_load', undefined));
    expect(document.documentElement.dataset.theme).toBe('light');

    load(savedWith({ theme: 'dark' }));
    await world;

    expect(document.documentElement.dataset.theme).toBe('dark');
  });
});

describe('Move to the Trash without asking', () => {
  const trashing = (answers: Answers = {}) => shown({ ops_trash: () => ({ undo: 4, failure: null }), ops_untrash: () => 1, ...answers });

  it('asks while the setting is on, and moves at once, with Undo, when it is off', async () => {
    const backend = await trashing();
    await key(listbox('Left'), 'F8');
    expect(screen.getByRole('dialog', { name: 'Move “a.pdf” to the Trash?' })).toBeInTheDocument();
    await key(screen.getByRole('button', { name: 'Cancel' }), 'Escape');
    preferences.set({ confirmTrash: false });

    await key(listbox('Left'), 'F8');

    await vi.waitFor(() => expect(backend.sent('ops_trash')).toEqual([{ path: '/left', names: ['a.pdf'] }]));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(await screen.findByText('Moved “a.pdf” to the Trash.')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Undo' }));
    expect(backend.sent('ops_untrash')).toEqual([{ token: 4 }]);
  });

  it('works the same for a selection, and a second press while it is on its way waits', async () => {
    let answer: (result: unknown) => void = () => {};
    const backend = await trashing({ ops_trash: () => new Promise((resolve) => (answer = resolve)) });
    preferences.set({ confirmTrash: false });
    await key(listbox('Left'), 'Insert');
    await key(listbox('Left'), 'Insert');

    await key(listbox('Left'), 'Delete');
    await key(listbox('Left'), 'Delete');
    answer({ undo: 5, failure: null });

    await vi.waitFor(() => expect(screen.getByText('Moved 2 items to the Trash.')).toBeInTheDocument());
    expect(backend.sent('ops_trash')).toHaveLength(1);
  });

  it('deleting for good still asks', async () => {
    const backend = await trashing();
    preferences.set({ confirmTrash: false });

    await key(listbox('Left'), 'F8', { shiftKey: true });

    expect(screen.getByRole('dialog', { name: 'Delete “a.pdf” permanently?' })).toBeInTheDocument();
    expect(backend.sent('ops_delete')).toEqual([]);
    expect(backend.sent('ops_trash')).toEqual([]);
  });

  it('a refusal is told in a card', async () => {
    await trashing({ ops_trash: () => Promise.reject(new IpcFailure({ code: 'forbidden', message: 'Not allowed here.' })) });
    preferences.set({ confirmTrash: false });

    await key(listbox('Left'), 'F8');

    expect(await screen.findByText('Not allowed here.')).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});

describe('Copy and move without the question', () => {
  it('F5 and F6 go to the other pane`s folder at once while the setting is off', async () => {
    const backend = await shown({ ops_start: () => 7 });
    preferences.set({ confirmTransfer: false });

    await key(listbox('Left'), 'F5');
    await vi.waitFor(() => expect(backend.sent('ops_start')).toHaveLength(1));
    await key(listbox('Left'), 'F6');
    await vi.waitFor(() => expect(backend.sent('ops_start')).toHaveLength(2));

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(backend.sent('ops_start')).toEqual([
      { kind: 'copy', from: '/left', names: ['a.pdf'], to: '/right', verify: false },
      { kind: 'move', from: '/left', names: ['a.pdf'], to: '/right', verify: false },
    ]);
    await vi.waitFor(() => expect(listbox('Left')).toHaveFocus());
  });

  it('the other way round when the right pane is active, and a started job takes the selection', async () => {
    const backend = await shown({ ops_start: () => 7 });
    preferences.set({ confirmTransfer: false });
    await key(listbox('Left'), 'Insert');
    expect(left().getByText(/^Selected:/)).toBeInTheDocument();

    await key(listbox('Left'), 'F5');
    await vi.waitFor(() => expect(left().queryByText(/^Selected:/)).not.toBeInTheDocument());
    await key(document.body, 'Tab');
    await key(listbox('Right'), 'ArrowDown');
    await key(listbox('Right'), 'F5');

    await vi.waitFor(() => expect(backend.sent('ops_start')).toHaveLength(2));
    expect(backend.sent('ops_start')[1]).toMatchObject({ kind: 'copy', from: '/right', names: ['c.md'], to: '/left' });
  });

  it('keeps asking while the setting is on', async () => {
    const backend = await shown();

    await key(listbox('Left'), 'F5');

    expect(screen.getByRole('dialog', { name: 'Copy “a.pdf” to' })).toBeInTheDocument();
    expect(backend.sent('ops_start')).toEqual([]);
  });

  it('a refusal of the backend (the same folder, say) is told in a card', async () => {
    await shown({ ops_start: () => Promise.reject(new IpcFailure({ code: 'invalidInput', message: 'That is the same folder.' })) });
    preferences.set({ confirmTransfer: false });

    await key(listbox('Left'), 'F5');

    expect(await screen.findByText('That is the same folder.')).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('what the dialog would refuse is refused as before: an archive that cannot be written into', async () => {
    const backend = await withArchiveOnRight(invoke, 'a.rar', { ops_start: () => 7 });
    preferences.set({ confirmTransfer: false });

    await key(listbox('Left'), 'F5');

    expect(await screen.findByText('Files can’t be added to this kind of archive.')).toBeInTheDocument();
    expect(backend.sent('ops_start')).toEqual([]);
    expect(backend.sent('ops_add_to_archive')).toEqual([]);
  });

  it('copying into an archive that can be written goes straight in too', async () => {
    const backend = await withArchiveOnRight(invoke, 'a.zip', { ops_add_to_archive: () => 8 });
    preferences.set({ confirmTransfer: false });

    await key(listbox('Left'), 'F5');

    await vi.waitFor(() => expect(backend.sent('ops_add_to_archive')).toEqual([{ archive: '/right/a.zip', dir: '', from: '/left', names: ['a.zip'] }]));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('asks when there is no folder to go to yet (the other pane has not listed)', async () => {
    const backend = appWorld(invoke);
    render(App);
    await backend.ready();
    backend.list('left', '/left', [entry('a.pdf')]);
    await tick();
    preferences.set({ confirmTransfer: false });
    await key(listbox('Left'), 'ArrowDown');

    await key(listbox('Left'), 'F5');

    expect(screen.getByRole('dialog', { name: 'Copy “a.pdf” to' })).toBeInTheDocument();
    expect(backend.sent('ops_start')).toEqual([]);
  });

  it('the dialogs that ask for something of their own are not skipped', async () => {
    await shown();
    preferences.set({ confirmTransfer: false });

    await key(listbox('Left'), 'F6', { shiftKey: true });

    expect(screen.getByRole('dialog', { name: 'Rename' })).toBeInTheDocument();
  });
});

describe('the update check at startup', () => {
  const info = { version: '9.9.9', currentVersion: '0.1.0', date: null, notes: null };

  it('is made, once the session is restored, by default', async () => {
    const backend = await shown({ update_check: () => info });

    await vi.waitFor(() => expect(backend.sent('update_check')).toHaveLength(1));
    expect(await screen.findByRole('button', { name: 'Install' })).toBeInTheDocument();
  });

  it('waits for the session, so that a setting it restores is respected', async () => {
    let load: (state: SavedState) => void = () => {};
    const backend = appWorld(invoke, { state_load: () => new Promise((resolve) => (load = resolve)), update_check: () => info });
    render(App);
    await vi.waitFor(() => expect(backend.sent('app_info')).toHaveLength(1));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(backend.sent('update_check')).toEqual([]); // the session has not said yet

    load(savedWith({ checkUpdatesAtStart: false }));
    await backend.ready();
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(backend.sent('update_check')).toEqual([]);
  });

  it('is not made when the state could not be read, as whether the user turned it off is not known', async () => {
    const backend = await shown({
      state_load: () => Promise.reject(new IpcFailure({ code: 'io', message: 'cannot read' })),
      update_check: () => info,
    });
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(backend.sent('update_check')).toEqual([]);
  });

  it('is not made when the setting is off, but the menu`s check still is', async () => {
    const backend = await shown({ state_load: () => savedWith({ checkUpdatesAtStart: false }), update_check: () => info });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(backend.sent('update_check')).toEqual([]);

    await fireEvent.click(screen.getByRole('button', { name: /^Settings/ }));
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Check for Updates…' }));

    await vi.waitFor(() => expect(backend.sent('update_check')).toHaveLength(1));
  });
});

describe('quitting while transfers run', () => {
  /** The window; the test sends the native menu's commands. */
  const windowWorld = async (answers: Answers = {}) => {
    let send: (command: MenuCommand) => void = () => {};
    const backend = await shown({ menu_subscribe: (args: { onCommand: typeof send }) => void (send = args.onCommand), ...answers });
    await vi.waitFor(() => expect(backend.sent('menu_subscribe')).toHaveLength(1));

    return { backend, send: (command: MenuCommand) => send(command), dirty: () => backend.sent('editor_set_dirty') };
  };

  const ask = () => screen.findByRole('dialog', { name: 'Quit Pairpane?' });

  it('tells the backend there is something to lose while transfers run, and not before or after', async () => {
    const { backend, dirty } = await windowWorld();
    expect(dirty()).toEqual([]);

    backend.ops({ kind: 'added', job: job(1) });
    await vi.waitFor(() => expect(dirty()).toEqual([{ dirty: true }]));
    backend.ops({ kind: 'ended', id: 1, outcome: { kind: 'done' }, skipped: 0, errors: [], moreErrors: 0 });

    await vi.waitFor(() => expect(dirty()).toEqual([{ dirty: true }, { dirty: false }]));
  });

  it('asks "Transfers are still running. Quit anyway?"; Quit closes the window', async () => {
    const { backend, send } = await windowWorld();
    backend.ops({ kind: 'added', job: job(1) });

    send('closeRequested');

    const dialog = await ask();
    expect(within(dialog).getByText('Transfers are still running. Quit anyway?')).toBeInTheDocument();
    expect(backend.sent('app_close_confirmed')).toHaveLength(0);
    expect(backend.sent('editor_close_ack')).toHaveLength(1);
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Quit' }));

    await vi.waitFor(() => expect(backend.sent('app_close_confirmed')).toHaveLength(1));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('Cancel keeps the app, and the backend is told again so that the next close asks again', async () => {
    const { backend, send, dirty } = await windowWorld();
    backend.ops({ kind: 'added', job: job(1) });
    await vi.waitFor(() => expect(dirty()).toHaveLength(1));

    send('closeRequested');
    await fireEvent.click(within(await ask()).getByRole('button', { name: 'Cancel' }));

    await vi.waitFor(() => expect(dirty()).toEqual([{ dirty: true }, { dirty: true }]));
    expect(backend.sent('app_close_confirmed')).toHaveLength(0);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('is not asked while the setting is off, or when nothing is running', async () => {
    const { backend, send, dirty } = await windowWorld();

    send('closeRequested');
    await vi.waitFor(() => expect(backend.sent('app_close_confirmed')).toHaveLength(1));

    preferences.set({ confirmQuitWithTransfers: false });
    backend.ops({ kind: 'added', job: job(1) });
    send('closeRequested');

    await vi.waitFor(() => expect(backend.sent('app_close_confirmed')).toHaveLength(2));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(dirty()).toEqual([]);
  });
});
