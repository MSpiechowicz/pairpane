import { fireEvent, screen, waitFor, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import type { SavedState } from '$lib/ipc/bindings/SavedState';
import { keymap } from '$lib/shell/keymap.svelte';

import { appShown, key, listbox, pane } from '../tests/app';
import { picture } from '../tests/thumbs';

const isTiles = (side: 'Left' | 'Right') => pane(`${side} pane`).querySelector('.row.tile') !== null;
const hasHeader = (side: 'Left' | 'Right') => within(pane(`${side} pane`)).queryByRole('group', { name: 'Sort by' }) !== null;

const menuButton = () => screen.getByRole('button', { name: 'View' });
const item = (name: 'Detailed' | 'Brief' | 'Thumbnails') => screen.getByRole('menuitemradio', { name: new RegExp(`^${name}`) });

/** The lists hold 3 rows of 3 tiles of 136 by 172 pixels (the test DOM measures nothing). */
function sized() {
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(3 * 172);
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(3 * 136);
}

const activeTab = () => [...pane('Left pane').querySelectorAll('.tab')].findIndex((tab) => tab.classList.contains('on'));

beforeEach(() => {
  vi.restoreAllMocks();
  sized();
  invoke.mockReset();
  window.localStorage.clear();
  keymap.resetAll();
});

describe('the thumbnails layout', () => {
  it('Ctrl+Shift+F1 shows the active pane as tiles, the other layouts keys bring the others back, the other pane untouched', async () => {
    await appShown(invoke);

    expect(await key(listbox('Left'), 'F1', { ctrlKey: true, shiftKey: true })).toBe(false); // handled: the default is prevented
    await tick();
    expect(isTiles('Left')).toBe(true);
    expect(hasHeader('Left')).toBe(false);
    expect(within(pane('Left pane')).getByRole('option', { name: /a\s?\.pdf/ })).toBeInTheDocument();
    expect(isTiles('Right')).toBe(false);
    expect(hasHeader('Right')).toBe(true);
    expect(listbox('Left')).toHaveFocus();

    await key(listbox('Left'), 'F1', { ctrlKey: true });
    await tick();
    expect(isTiles('Left')).toBe(false);
    expect(pane('Left pane').querySelector('.row.brief')).not.toBeNull();

    await key(listbox('Left'), 'F1', { ctrlKey: true, shiftKey: true });
    await key(listbox('Left'), 'F2', { ctrlKey: true });
    await tick();
    expect(isTiles('Left')).toBe(false);
    expect(hasHeader('Left')).toBe(true);
  });

  it('the cursor and the selection stay as they are when the layout changes', async () => {
    await appShown(invoke); // the cursor is on a.pdf
    await key(listbox('Left'), ' '); // selects a.pdf and moves down to b.txt

    await key(listbox('Left'), 'F1', { ctrlKey: true, shiftKey: true });
    await tick();

    expect(within(pane('Left pane')).getByRole('option', { name: /a\s?\.pdf/, selected: true })).toBeInTheDocument();
    expect(document.querySelector('.row.cursor')).toHaveTextContent('b.txt');
    expect(listbox('Left')).toHaveAttribute('aria-activedescendant', document.querySelector('.row.cursor')?.id);
  });

  it('the palette lists it with its key and switches the active pane', async () => {
    await appShown(invoke);
    await key(listbox('Left'), 'Tab'); // the right pane
    await key(listbox('Left'), 'k', { ctrlKey: true });
    const field = screen.getByRole('combobox', { name: 'Command, folder or name' });

    await fireEvent.input(field, { target: { value: 'thumbnails view' } });
    expect(screen.getByRole('option', { name: /Thumbnails view/ })).toHaveTextContent('Ctrl+Shift+F1');
    await fireEvent.keyDown(field, { key: 'Enter' });
    await tick();

    expect(isTiles('Right')).toBe(true);
    expect(isTiles('Left')).toBe(false);
  });

  it('the View menu has a Thumbnails item with its key, marks the active layout and switches it', async () => {
    await appShown(invoke);

    await fireEvent.click(menuButton());
    expect(item('Thumbnails')).toHaveAttribute('aria-checked', 'false');
    expect(item('Thumbnails')).toHaveAttribute('aria-keyshortcuts', 'Ctrl+Shift+F1');
    expect(item('Detailed')).toHaveAttribute('aria-checked', 'true');

    await fireEvent.click(item('Thumbnails'));
    await tick();

    expect(screen.queryByRole('menu', { name: 'View' })).not.toBeInTheDocument();
    expect(isTiles('Left')).toBe(true);
    expect(isTiles('Right')).toBe(false);

    await fireEvent.click(menuButton());
    expect(item('Thumbnails')).toHaveAttribute('aria-checked', 'true');
    expect(item('Detailed')).toHaveAttribute('aria-checked', 'false');
    expect(item('Brief')).toHaveAttribute('aria-checked', 'false');
  });

  it('the key is the keymap`s: set to another key, the default one does nothing and the new one switches', async () => {
    await appShown(invoke);
    keymap.set('layoutThumbnails', { key: 'F9', ctrl: true, shift: true });

    await key(listbox('Left'), 'F1', { ctrlKey: true, shiftKey: true });
    await tick();
    expect(isTiles('Left')).toBe(false);

    await key(listbox('Left'), 'F9', { ctrlKey: true, shiftKey: true });
    await tick();
    expect(isTiles('Left')).toBe(true);
  });

  it('every tab has its own layout, and a new tab starts with the layout of the tab it was made from', async () => {
    await appShown(invoke);
    await key(listbox('Left'), 'F1', { ctrlKey: true, shiftKey: true });

    await key(listbox('Left'), 't', { ctrlKey: true });
    await tick();
    expect(isTiles('Left')).toBe(true);
    await key(listbox('Left'), 'F2', { ctrlKey: true });
    await tick();
    expect(isTiles('Left')).toBe(false);
    expect(activeTab()).toBe(1);

    await key(listbox('Left'), 'Tab', { ctrlKey: true }); // back to the first tab
    await tick();
    expect(activeTab()).toBe(0);
    expect(isTiles('Left')).toBe(true);
  });

  it('the layout of every tab is saved with the session, as the saved state names it', async () => {
    const backend = await appShown(invoke);
    await key(listbox('Left'), 'F1', { ctrlKey: true, shiftKey: true });
    await key(listbox('Left'), 't', { ctrlKey: true });
    await key(listbox('Left'), 'F2', { ctrlKey: true });

    await waitFor(() => {
      const saved = (backend.sent('state_save').at(-1) as { state: SavedState }).state;
      expect(saved.left.tabs.map((tab) => tab.layout)).toEqual(['thumbnails', undefined]);
    });
  });

  it('a saved thumbnails layout comes back with the tab', async () => {
    const state = {
      active: 'left',
      left: { tabs: [{ path: '/left', sort: 'name', desc: false, layout: 'thumbnails' }], active: 0 },
      right: { tabs: [{ path: '/right', sort: 'name', desc: false }], active: 0 },
      hotlist: [],
    } as unknown as SavedState;

    await appShown(invoke, { state_load: () => state });

    expect(isTiles('Left')).toBe(true);
    expect(isTiles('Right')).toBe(false);
    expect(invoke.mock.calls.filter(([name]) => name === 'state_save')).toEqual([]); // loading is not a change
  });

  it('asks for no thumbnail in a folder without pictures', async () => {
    const backend = await appShown(invoke, { thumb_get: () => picture(2, 2) });

    await key(listbox('Left'), 'F1', { ctrlKey: true, shiftKey: true });
    await tick();

    expect(isTiles('Left')).toBe(true);
    expect(backend.sent('thumb_get')).toEqual([]);
  });

  it('the inline rename works in a tile: F2 opens the field in it, Escape closes it', async () => {
    await appShown(invoke);
    await key(listbox('Left'), 'F1', { ctrlKey: true, shiftKey: true });
    await tick();

    await key(listbox('Left'), 'F2');
    const field = await screen.findByRole('textbox', { name: 'New name' });
    expect(field.closest('.row')).toHaveClass('tile');
    expect(field).toHaveValue('a.pdf');

    await key(field, 'Escape');
    expect(screen.queryByRole('textbox', { name: 'New name' })).not.toBeInTheDocument();
  });
});
