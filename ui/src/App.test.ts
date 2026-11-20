import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

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
});
