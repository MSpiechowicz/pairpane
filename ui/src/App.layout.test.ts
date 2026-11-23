import { fireEvent, screen, waitFor, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import type { SavedState } from '$lib/ipc/bindings/SavedState';
import { keymap } from '$lib/shell/keymap.svelte';

import { appShown, key, listbox, pane } from '../tests/app';

const left = () => within(pane('Left pane'));
const right = () => within(pane('Right pane'));
const isBrief = (side: 'Left' | 'Right') => pane(`${side} pane`).querySelector('.row.brief') !== null;
const hasHeader = (side: 'Left' | 'Right') => within(pane(`${side} pane`)).queryByRole('group', { name: 'Sort by' }) !== null;

const menuButton = () => screen.getByRole('button', { name: 'View' });
const item = (name: 'Detailed' | 'Brief') => screen.getByRole('menuitemradio', { name: new RegExp(`^${name}`) });

/** The lists hold 10 rows of 24px and 3 columns of 200px (the test DOM measures nothing). */
function sized() {
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(240);
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(600);
}

/** The position of the active tab among the tabs of the left pane. */
const activeTab = () => [...pane('Left pane').querySelectorAll('.tab')].findIndex((tab) => tab.classList.contains('on'));

beforeEach(() => {
  vi.restoreAllMocks();
  sized();
  invoke.mockReset();
  window.localStorage.clear();
  keymap.resetAll();
});

describe('the pane layouts', () => {
  it('start detailed; Ctrl+F1 shows the active pane in brief, Ctrl+F2 in detail again, the other pane untouched', async () => {
    await appShown(invoke);
    expect(isBrief('Left')).toBe(false);
    expect(hasHeader('Left')).toBe(true);

    expect(await key(listbox('Left'), 'F1', { ctrlKey: true })).toBe(false); // handled: the default is prevented
    await tick();
    expect(isBrief('Left')).toBe(true);
    expect(hasHeader('Left')).toBe(false);
    expect(left().getByRole('option', { name: /a\s?\.pdf/ })).toBeInTheDocument();
    expect(isBrief('Right')).toBe(false);
    expect(hasHeader('Right')).toBe(true);
    expect(listbox('Left')).toHaveFocus();

    await key(listbox('Left'), 'F2', { ctrlKey: true });
    await tick();
    expect(isBrief('Left')).toBe(false);
    expect(hasHeader('Left')).toBe(true);
  });

  it('the cursor and the selection stay as they are when the layout changes', async () => {
    await appShown(invoke); // the cursor is on a.pdf
    await key(listbox('Left'), ' '); // selects a.pdf and moves down to b.txt

    await key(listbox('Left'), 'F1', { ctrlKey: true });
    await tick();

    expect(left().getByRole('option', { name: /a\s?\.pdf/, selected: true })).toBeInTheDocument();
    expect(listbox('Left')).toHaveAttribute('aria-activedescendant', document.querySelector('.row.cursor')?.id);
    expect(document.querySelector('.row.cursor')).toHaveTextContent('b.txt');
  });

  it('the palette lists both views with their keys and switches the active pane', async () => {
    await appShown(invoke);
    await key(listbox('Left'), 'Tab'); // the right pane
    await key(listbox('Left'), 'k', { ctrlKey: true });
    const field = screen.getByRole('combobox', { name: 'Command, folder or name' });

    await fireEvent.input(field, { target: { value: 'brief view' } });
    expect(screen.getByRole('option', { name: /Brief view/ })).toHaveTextContent('Ctrl+F1');
    await fireEvent.keyDown(field, { key: 'Enter' });
    await tick();

    expect(isBrief('Right')).toBe(true);
    expect(isBrief('Left')).toBe(false);

    await key(listbox('Right'), 'k', { ctrlKey: true });
    await fireEvent.input(screen.getByRole('combobox', { name: 'Command, folder or name' }), { target: { value: 'detailed view' } });
    expect(screen.getByRole('option', { name: /Detailed view/ })).toHaveTextContent('Ctrl+F2');
    await fireEvent.keyDown(screen.getByRole('combobox', { name: 'Command, folder or name' }), { key: 'Enter' });
    await tick();

    expect(isBrief('Right')).toBe(false);
  });

  it('the View menu has Detailed and Brief with their keys, marks the active layout and switches it', async () => {
    await appShown(invoke);

    await fireEvent.click(menuButton());
    expect(screen.getByRole('menu', { name: 'View' })).toBeInTheDocument();
    expect(item('Detailed')).toHaveAttribute('aria-checked', 'true');
    expect(item('Detailed')).toHaveAttribute('aria-keyshortcuts', 'Ctrl+F2');
    expect(item('Brief')).toHaveAttribute('aria-checked', 'false');
    expect(item('Brief')).toHaveAttribute('aria-keyshortcuts', 'Ctrl+F1');

    await fireEvent.click(item('Brief'));
    await tick();

    expect(screen.queryByRole('menu', { name: 'View' })).not.toBeInTheDocument();
    expect(isBrief('Left')).toBe(true);
    expect(isBrief('Right')).toBe(false);

    await fireEvent.click(menuButton());
    expect(item('Brief')).toHaveAttribute('aria-checked', 'true');
    expect(item('Detailed')).toHaveAttribute('aria-checked', 'false');
  });

  it('the View menu follows the active pane, and a click outside or Escape closes it', async () => {
    await appShown(invoke);
    await key(listbox('Left'), 'F1', { ctrlKey: true });
    await key(listbox('Left'), 'Tab');

    await fireEvent.click(menuButton());
    expect(item('Detailed')).toHaveAttribute('aria-checked', 'true'); // the right pane is detailed

    await fireEvent.keyDown(document.body, { key: 'Escape' });
    expect(screen.queryByRole('menu', { name: 'View' })).not.toBeInTheDocument();

    await fireEvent.click(menuButton());
    await fireEvent.click(document.body);
    expect(screen.queryByRole('menu', { name: 'View' })).not.toBeInTheDocument();
  });

  it('every tab has its own layout; a new tab starts with the layout of the tab it was made from', async () => {
    await appShown(invoke);
    await key(listbox('Left'), 'F1', { ctrlKey: true });

    await key(listbox('Left'), 't', { ctrlKey: true });
    await tick();
    expect(isBrief('Left')).toBe(true);
    await key(listbox('Left'), 'F2', { ctrlKey: true });
    await tick();
    expect(isBrief('Left')).toBe(false);
    expect(activeTab()).toBe(1);

    await key(listbox('Left'), 'Tab', { ctrlKey: true }); // back to the first tab
    await tick();
    expect(activeTab()).toBe(0);
    expect(isBrief('Left')).toBe(true);

    await key(listbox('Left'), 'Tab', { ctrlKey: true });
    await tick();
    expect(activeTab()).toBe(1);
    expect(isBrief('Left')).toBe(false);
  });

  it('the layout of every tab is saved with the session', async () => {
    const backend = await appShown(invoke);
    await key(listbox('Left'), 'F1', { ctrlKey: true });
    await key(listbox('Left'), 't', { ctrlKey: true }); // a second tab, brief too
    await key(listbox('Left'), 'F2', { ctrlKey: true });

    await waitFor(() => {
      const saved = (backend.sent('state_save').at(-1) as { state: SavedState }).state;
      expect(saved.left.tabs.map((tab) => tab.layout)).toEqual(['brief', undefined]);
      expect(saved.right.tabs.map((tab) => tab.layout)).toEqual([undefined]);
    });
  });

  it('a saved layout comes back with the tab; a file from before layouts shows every tab in detail', async () => {
    const state: SavedState = {
      active: 'left',
      left: { tabs: [{ path: '/left', sort: 'name', desc: false, layout: 'brief' }], active: 0 },
      right: { tabs: [{ path: '/right', sort: 'name', desc: false }], active: 0 },
      hotlist: [],
    };

    await appShown(invoke, { state_load: () => state });
    expect(isBrief('Left')).toBe(true);
    expect(isBrief('Right')).toBe(false);
    expect(right().getByRole('group', { name: 'Sort by' })).toBeInTheDocument();
    expect(invoke.mock.calls.filter(([name]) => name === 'state_save')).toEqual([]); // loading is not a change
  });

  it('the keys are the keymap`s: set to another key, the default one does nothing and the new one switches', async () => {
    await appShown(invoke);
    keymap.set('layoutBrief', { key: 'F9', ctrl: true });

    await key(listbox('Left'), 'F1', { ctrlKey: true });
    await tick();
    expect(isBrief('Left')).toBe(false);

    await key(listbox('Left'), 'F9', { ctrlKey: true });
    await tick();
    expect(isBrief('Left')).toBe(true);
  });

  it('the inline rename works in brief: F2 opens the field in the item, Escape closes it', async () => {
    await appShown(invoke);
    await key(listbox('Left'), 'F1', { ctrlKey: true });
    await tick();

    await key(listbox('Left'), 'F2');
    const field = await screen.findByRole('textbox', { name: 'New name' });
    expect(field.closest('.row')).toHaveClass('brief');
    expect(field).toHaveValue('a.pdf');

    await key(field, 'Escape');
    expect(screen.queryByRole('textbox', { name: 'New name' })).not.toBeInTheDocument();
  });
});
