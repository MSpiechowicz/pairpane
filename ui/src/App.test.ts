import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import type { ListEvent } from '$lib/ipc/bindings/ListEvent';
import type { SavedState } from '$lib/ipc/bindings/SavedState';
import type { VolumeEvent } from '$lib/ipc/bindings/VolumeEvent';

import App from './App.svelte';

/** app_info answers with `info`; every other command (menu_subscribe, update_check) finds nothing. */
function backend(info: () => Promise<unknown>) {
  invoke.mockImplementation((name: string) => (name === 'app_info' ? info() : Promise.resolve(null)));
}

const pane = (name: string) => screen.getByRole('region', { name });

describe('App', () => {
  beforeEach(() => {
    invoke.mockReset();
    vi.restoreAllMocks();
    window.localStorage.clear();
  });

  it('shows the shell: top bar with the version, two panes and the dock', async () => {
    backend(() => Promise.resolve({ name: 'Pairpane', version: '0.1.0' }));

    render(App);

    expect(screen.getByRole('heading', { name: 'Pairpane' })).toBeInTheDocument();
    expect(await screen.findByText('v0.1.0')).toBeInTheDocument();
    expect(pane('Left pane')).toBeInTheDocument();
    expect(pane('Right pane')).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Shell command' })).toBeInTheDocument();
    for (const key of ['View (F3)', 'Edit (F4)', 'Copy (F5)', 'Move (F6)', 'New folder (F7)', 'Delete (F8)']) {
      expect(screen.getByRole('button', { name: key })).toBeInTheDocument();
    }
  });

  it('shows a failure message when the backend call is rejected', async () => {
    backend(() => Promise.reject(new Error('The desktop runtime is not available.')));

    render(App);

    expect(await screen.findByRole('alert')).toHaveTextContent('The desktop runtime is not available.');
    expect(screen.queryByText(/^v\d/)).not.toBeInTheDocument();
  });

  it('Tab switches the active pane from a pane or the page; elsewhere it moves focus as usual', async () => {
    backend(() => Promise.resolve({ name: 'Pairpane', version: '0.1.0' }));
    render(App);

    expect(pane('Left pane')).toHaveAttribute('aria-current', 'true');

    await fireEvent.keyDown(document.body, { key: 'Tab' });
    expect(pane('Right pane')).toHaveAttribute('aria-current', 'true');
    expect(pane('Left pane')).not.toHaveAttribute('aria-current');

    const newTab = within(pane('Right pane')).getByRole('button', { name: 'New tab' });
    expect(await fireEvent.keyDown(newTab, { key: 'Tab' })).toBe(false); // default prevented: focus stays put
    expect(pane('Left pane')).toHaveAttribute('aria-current', 'true');

    for (const target of [
      screen.getByRole('textbox', { name: 'Shell command' }),
      screen.getByRole('button', { name: 'Settings' }),
      screen.getByRole('button', { name: 'Copy (F5)' }),
    ]) {
      expect(await fireEvent.keyDown(target, { key: 'Tab' })).toBe(true);
    }
    await fireEvent.keyDown(document.body, { key: 'Tab', shiftKey: true });
    expect(pane('Left pane')).toHaveAttribute('aria-current', 'true');

    await fireEvent.click(pane('Right pane'));
    expect(pane('Right pane')).toHaveAttribute('aria-current', 'true');
  });

  it('the active pane`s list has the focus, and Tab in the mask prompt moves focus as usual', async () => {
    backend(() => Promise.resolve({ name: 'Pairpane', version: '0.1.0' }));
    render(App);
    const lists = screen.getAllByRole('listbox');
    expect(lists[0]).toHaveFocus();

    await fireEvent.keyDown(lists[0], { key: 'Tab' });
    expect(pane('Right pane')).toHaveAttribute('aria-current', 'true');
    expect(lists[1]).toHaveFocus();

    await fireEvent.keyDown(lists[1], { key: '+' });
    const mask = await screen.findByRole('textbox', { name: /Select/ });
    expect(await fireEvent.keyDown(mask, { key: 'Tab' })).toBe(true);
    expect(pane('Right pane')).toHaveAttribute('aria-current', 'true');
  });

  it('Settings > Check for Updates… runs a manual update check', async () => {
    backend(() => Promise.resolve({ name: 'Pairpane', version: '0.1.0' }));
    render(App);
    await screen.findByText('v0.1.0');

    await fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Check for Updates…' }));

    expect(await screen.findByText(/Pairpane is up to date/)).toBeInTheDocument();
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('the Settings menu closes on Escape and on a click outside', async () => {
    backend(() => Promise.resolve({ name: 'Pairpane', version: '0.1.0' }));
    render(App);
    const settings = screen.getByRole('button', { name: 'Settings' });

    await fireEvent.click(settings);
    expect(screen.getByRole('menu')).toBeInTheDocument();
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();

    await fireEvent.click(settings);
    await fireEvent.click(pane('Left pane'));
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('on macOS the top bar has no Settings menu (the native menu has the update check) and shows ⌘K', () => {
    vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)');
    backend(() => Promise.resolve({ name: 'Pairpane', version: '0.1.0' }));
    render(App);

    expect(screen.queryByRole('button', { name: 'Settings' })).not.toBeInTheDocument();
    expect(screen.getByText('⌘K')).toBeInTheDocument();
  });

  it('the theme button cycles System → Light → Dark', async () => {
    backend(() => Promise.resolve({ name: 'Pairpane', version: '0.1.0' }));
    render(App);

    const button = screen.getByRole('button', { name: /Theme: System/ });
    await fireEvent.click(button);
    expect(button).toHaveAccessibleName(/Theme: Light/);
    expect(document.documentElement.dataset.theme).toBe('light');

    await fireEvent.click(button);
    expect(button).toHaveAccessibleName(/Theme: Dark/);
    expect(document.documentElement.dataset.theme).toBe('dark');
  });

  describe('tab and history keys', () => {
    /** A backend that remembers the channel of every `pane_open`; `list` delivers a folder on the left pane's latest. */
    function folders() {
      const opens: { args: { pane: string; path: string | null }; emit: (event: ListEvent) => void }[] = [];
      invoke.mockImplementation((name: string, args: { pane: string; path: string | null; onEvent: never }) => {
        if (name === 'pane_open') {
          opens.push({ args, emit: args.onEvent });
        }
        return Promise.resolve(name === 'app_info' ? { name: 'Pairpane', version: '0.1.0' } : null);
      });

      const list = async (path: string) => {
        // The panes open their folders once the saved session has been looked at.
        await vi.waitFor(() => expect(opens.some((open) => open.args.pane === 'left')).toBe(true));
        const channel = opens.filter((open) => open.args.pane === 'left').at(-1)!;
        const names = path.split('/').filter((part) => part !== '');
        channel.emit({
          kind: 'start',
          path,
          parent: path === '/' ? null : '/',
          crumbs: [
            { name: '/', path: '/' },
            ...names.map((part, index) => ({ name: part, path: `/${names.slice(0, index + 1).join('/')}` })),
          ],
          total: 0,
        });
        channel.emit({ kind: 'done', dirs: 0, files: 0, bytes: 0, free: null });
        await tick();
      };

      return { opens, list, last: () => opens[opens.length - 1] };
    }

    const closeButtons = (side: string) => within(pane(side)).queryAllByRole('button', { name: /^Close / });

    it('Ctrl+T adds a tab to the active pane and Ctrl+W closes it; the last tab stays', async () => {
      folders();
      render(App);
      expect(closeButtons('Left pane')).toHaveLength(0);

      expect(await fireEvent.keyDown(document.body, { key: 't', ctrlKey: true })).toBe(false);
      expect(closeButtons('Left pane')).toHaveLength(1);
      expect(closeButtons('Right pane')).toHaveLength(0);

      expect(await fireEvent.keyDown(document.body, { key: 'w', ctrlKey: true })).toBe(false);
      expect(closeButtons('Left pane')).toHaveLength(0);

      await fireEvent.keyDown(document.body, { key: 'w', ctrlKey: true });
      expect(within(pane('Left pane')).getByRole('navigation', { name: 'Path' })).toBeInTheDocument();
    });

    it('Ctrl+Tab and Ctrl+Shift+Tab switch tabs, opening the folder of the tab; plain Tab still switches panes', async () => {
      const backend = folders();
      render(App);
      await backend.list('/a');
      await fireEvent.keyDown(document.body, { key: 't', ctrlKey: true });
      await fireEvent.click(within(pane('Left pane')).getByRole('button', { name: 'System' }));
      await backend.list('/');

      expect(await fireEvent.keyDown(document.body, { key: 'Tab', ctrlKey: true })).toBe(false);
      expect(backend.last().args).toMatchObject({ pane: 'left', path: '/a' });
      await backend.list('/a');

      expect(await fireEvent.keyDown(document.body, { key: 'Tab', ctrlKey: true, shiftKey: true })).toBe(false);
      expect(backend.last().args).toMatchObject({ pane: 'left', path: '/' });
      expect(pane('Left pane')).toHaveAttribute('aria-current', 'true');

      await fireEvent.keyDown(document.body, { key: 'Tab' });
      expect(pane('Right pane')).toHaveAttribute('aria-current', 'true');
    });

    it('Alt+Left and Alt+Right walk the active pane`s history and keep the webview from navigating', async () => {
      const backend = folders();
      render(App);
      await backend.list('/a');
      await fireEvent.click(within(pane('Left pane')).getByRole('button', { name: 'System' }));
      await backend.list('/');

      expect(await fireEvent.keyDown(document.body, { key: 'ArrowLeft', altKey: true })).toBe(false);
      expect(backend.last().args).toMatchObject({ pane: 'left', path: '/a' });
      await backend.list('/a');

      expect(await fireEvent.keyDown(document.body, { key: 'ArrowRight', altKey: true })).toBe(false);
      expect(backend.last().args).toMatchObject({ pane: 'left', path: '/' });

      // Nothing further ahead: still not the webview's business.
      await backend.list('/');
      expect(await fireEvent.keyDown(document.body, { key: 'ArrowRight', altKey: true })).toBe(false);
    });

    it('the shortcuts act on the active pane', async () => {
      folders();
      render(App);

      await fireEvent.keyDown(document.body, { key: 'Tab' });
      await fireEvent.keyDown(document.body, { key: 't', ctrlKey: true });

      expect(closeButtons('Right pane')).toHaveLength(1);
      expect(closeButtons('Left pane')).toHaveLength(0);
    });

    it('are left alone in a text field and when something else already handled the key', async () => {
      folders();
      render(App);
      const shell = screen.getByRole('textbox', { name: 'Shell command' });

      expect(await fireEvent.keyDown(shell, { key: 't', ctrlKey: true })).toBe(true);
      expect(await fireEvent.keyDown(shell, { key: 'ArrowLeft', altKey: true })).toBe(true);
      expect(closeButtons('Left pane')).toHaveLength(0);

      const handled = (event: KeyboardEvent) => event.preventDefault();
      document.body.addEventListener('keydown', handled);
      await fireEvent.keyDown(document.body, { key: 't', ctrlKey: true });
      document.body.removeEventListener('keydown', handled);
      expect(closeButtons('Left pane')).toHaveLength(0);
    });
  });

  describe('drive menu, hotlist and the saved session', () => {
    const SAVED: SavedState = {
      active: 'right',
      left: { tabs: [{ path: '/saved/left', sort: 'size', desc: true }], active: 0 },
      right: { tabs: [{ path: '/saved/right', sort: 'name', desc: false }], active: 0 },
      hotlist: ['/saved/favourite'],
    };

    /** A backend that has a saved session (or none), a drive list, and a record of what was asked of it. */
    function world(state: SavedState | null = null) {
      const opens: { args: { pane: string; path: string | null }; emit: (event: ListEvent) => void }[] = [];
      const lists: { emit: (event: VolumeEvent) => void }[] = [];
      invoke.mockImplementation((name: string, args: { onEvent: never }) => {
        if (name === 'pane_open') {
          opens.push({ args: args as never, emit: args.onEvent });
        } else if (name === 'volumes_list') {
          lists.push({ emit: args.onEvent });
        }

        const answers: Record<string, unknown> = { app_info: { name: 'Pairpane', version: '0.1.0' }, state_load: state };
        return Promise.resolve(answers[name] ?? null);
      });

      const started = () => vi.waitFor(() => expect(opens.length).toBeGreaterThanOrEqual(2));
      const list = (side: string, path: string) => {
        const channel = opens.filter((open) => open.args.pane === side).at(-1)!;
        const names = path.split('/').filter((part) => part !== '');
        channel.emit({
          kind: 'start',
          path,
          parent: '/',
          crumbs: [
            { name: '/', path: '/' },
            ...names.map((part, index) => ({ name: part, path: `/${names.slice(0, index + 1).join('/')}` })),
          ],
          total: 0,
        });
        channel.emit({ kind: 'done', dirs: 0, files: 0, bytes: 0, free: null });
      };
      const sent = (name: string) => invoke.mock.calls.filter(([command]) => command === name);
      const drives = () =>
        lists[0]?.emit({
          kind: 'list',
          volumes: [
            { name: 'System', label: null, path: '/', kind: 'system', free: null },
            { name: 'USB', label: null, path: '/media/USB', kind: 'removable', free: null },
          ],
        });

      return { opens, started, list, sent, drives };
    }

    const driveButton = (side: string) => within(pane(side)).getByRole('button', { name: 'Drives' });

    it('Alt+F1 and Alt+F2 open the drive menu of that side and make it the active pane', async () => {
      const backend = world();
      render(App);
      await backend.started();
      await tick();
      backend.drives();
      await tick();

      expect(await fireEvent.keyDown(document.body, { key: 'F2', altKey: true })).toBe(false);
      expect(within(pane('Right pane')).getByRole('listbox', { name: 'Drives' })).toBeInTheDocument();
      expect(pane('Right pane')).toHaveAttribute('aria-current', 'true');
      expect(screen.getAllByRole('listbox', { name: 'Drives' })).toHaveLength(1);

      await fireEvent.keyDown(document.body, { key: 'F1', altKey: true });
      expect(within(pane('Left pane')).getByRole('listbox', { name: 'Drives' })).toBeInTheDocument();
      expect(pane('Left pane')).toHaveAttribute('aria-current', 'true');
      expect(screen.getAllByRole('listbox', { name: 'Drives' })).toHaveLength(1);
    });

    it('the drive icon opens and closes the menu; choosing a drive opens it in that pane and refocuses the list', async () => {
      const backend = world();
      render(App);
      await backend.started();
      backend.drives();
      await tick();

      await fireEvent.click(driveButton('Left pane'));
      expect(driveButton('Left pane')).toHaveAttribute('aria-expanded', 'true');
      await fireEvent.click(driveButton('Left pane'));
      expect(screen.queryByRole('listbox', { name: 'Drives' })).not.toBeInTheDocument();

      await fireEvent.click(driveButton('Right pane'));
      await fireEvent.click(screen.getByRole('option', { name: /USB/ }));

      expect(backend.opens.at(-1)?.args).toMatchObject({ pane: 'right', path: '/media/USB' });
      expect(screen.queryByRole('listbox', { name: 'Drives' })).not.toBeInTheDocument();
      expect(within(pane('Right pane')).getByRole('listbox', { name: 'Right pane files' })).toHaveFocus();
    });

    it('Tab in a menu moves focus as usual instead of switching panes; Escape returns to the list', async () => {
      const backend = world();
      render(App);
      await backend.started();
      await fireEvent.keyDown(document.body, { key: 'd', ctrlKey: true });

      const field = screen.getByRole('combobox', { name: 'Filter favourite folders' });
      expect(await fireEvent.keyDown(field, { key: 'Tab' })).toBe(true);
      expect(pane('Left pane')).toHaveAttribute('aria-current', 'true');

      await fireEvent.keyDown(field, { key: 'Escape' });
      expect(screen.queryByRole('combobox')).not.toBeInTheDocument();
      expect(within(pane('Left pane')).getByRole('listbox', { name: 'Left pane files' })).toHaveFocus();
    });

    it('Ctrl+D opens the hotlist under the active pane; typing in it does not run pane shortcuts', async () => {
      const backend = world(SAVED);
      render(App);
      await backend.started();
      await tick();

      expect(await fireEvent.keyDown(document.body, { key: 'd', ctrlKey: true })).toBe(false);
      const field = within(pane('Right pane')).getByRole('combobox', { name: 'Filter favourite folders' });
      expect(field).toHaveFocus();
      expect(within(pane('Right pane')).getByRole('option', { name: '/saved/favourite' })).toBeInTheDocument();

      expect(await fireEvent.keyDown(field, { key: 't', ctrlKey: true })).toBe(true);
      expect(within(pane('Right pane')).queryAllByRole('button', { name: /^Close / })).toHaveLength(0);
    });

    it('a favourite added to the hotlist is there the next time the menu opens, for either pane', async () => {
      const backend = world();
      render(App);
      await backend.started();
      backend.list('left', '/a/b');
      await tick();

      await fireEvent.keyDown(document.body, { key: 'd', ctrlKey: true });
      await fireEvent.click(screen.getByRole('button', { name: 'Add current folder' }));
      await fireEvent.keyDown(screen.getByRole('combobox'), { key: 'Escape' });

      await fireEvent.keyDown(document.body, { key: 'Tab' });
      await fireEvent.keyDown(document.body, { key: 'd', ctrlKey: true });
      expect(within(pane('Right pane')).getByRole('option', { name: '/a/b' })).toBeInTheDocument();
    });

    it('reads the drives once at startup', async () => {
      const backend = world();
      render(App);
      await backend.started();

      expect(backend.sent('volumes_list')).toHaveLength(1);
    });

    it('opens the folders of the saved session, and the saved pane is the active one', async () => {
      const backend = world(SAVED);
      render(App);
      await backend.started();

      expect(backend.opens.map((open) => [open.args.pane, open.args.path])).toEqual([
        ['left', '/saved/left'],
        ['right', '/saved/right'],
      ]);
      expect(pane('Right pane')).toHaveAttribute('aria-current', 'true');
      expect(within(pane('Right pane')).getByRole('listbox', { name: 'Right pane files' })).toHaveFocus();
    });

    it('opens the home folder in both panes when nothing was saved', async () => {
      const backend = world();
      render(App);
      await backend.started();

      expect(backend.opens.map((open) => open.args.path)).toEqual([null, null]);
    });

    it('saves the session once things have settled, not on every step', async () => {
      const backend = world();
      render(App);
      await backend.started();
      backend.list('left', '/a');
      backend.list('right', '/b');
      await tick();
      expect(backend.sent('state_save')).toHaveLength(0);

      await vi.waitFor(() => expect(backend.sent('state_save')).toHaveLength(1), { timeout: 2000 });
      expect(backend.sent('state_save')[0][1]).toMatchObject({
        state: { active: 'left', left: { tabs: [{ path: '/a' }] }, right: { tabs: [{ path: '/b' }] } },
      });
    });
  });
});
