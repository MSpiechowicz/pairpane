import { cleanup, fireEvent, render, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { Hotlist } from '$lib/panes/hotlist.svelte';
import { PaneTabs } from '$lib/panes/tabs.svelte';
import { Volumes } from '$lib/panes/volumes.svelte';

import { entry, fakeBackend } from '../../../tests/panes';
import type { Menu } from './menu';
import Pane from './Pane.svelte';
import { preferences } from './preferences.svelte';

const TOTAL = 100_000;

async function huge(active = true, menu?: Menu) {
  const backend = fakeBackend();
  const tabs = new PaneTabs(backend.ipc, 'left');
  const view = tabs.view;
  await view.listing.open('/big');
  backend.last().emit({
    kind: 'start',
    path: '/big',
    parent: '/',
    crumbs: [
      { name: '/', path: '/' },
      { name: 'big', path: '/big' },
    ],
    total: TOTAL,
  });
  backend.last().emit({
    kind: 'batch',
    offset: 0,
    entries: [entry('sub', 'dir'), ...Array.from({ length: 199 }, (_, i) => entry(`f${i}.txt`, 'file', { size: 10 }))],
  });

  const onActivate = vi.fn();
  const volumes = new Volumes(backend.ipc);
  const hotlist = new Hotlist();
  const onMenu = vi.fn();
  render(Pane, { label: 'Left pane', tabs, volumes, hotlist, menu, active, onActivate, onMenu });
  return { backend, tabs, view, onActivate, onMenu };
}

afterEach(() => vi.restoreAllMocks());

/** A list tall enough for 10 rows (the layout engine of the test DOM measures nothing). */
const tenRowsTall = () => vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(240);

const list = () => screen.getByRole('listbox');
const viewport = () => document.querySelector('.viewport') as HTMLElement;
const cursorRow = () => document.querySelector('.row.cursor') as HTMLElement;

describe('Pane', () => {
  it('renders only the rows in view of a 100 000-entry folder', async () => {
    await huge();

    const rows = document.querySelectorAll('.row');
    expect(rows.length).toBeGreaterThan(0);
    expect(rows.length).toBeLessThan(60);
    expect(screen.getByText('..')).toBeInTheDocument();
    expect(screen.getByText('sub')).toBeInTheDocument();
    expect((document.querySelector('.spacer') as HTMLElement).style.height).toBe(`${(TOTAL + 1) * 24}px`);
  });

  it('a name that would pass for another shows its bidirectional and control characters written out', async () => {
    const { backend } = await huge();

    backend.last().emit({ kind: 'batch', offset: 0, entries: [entry('report\u202Egpj.exe', 'file')] });
    await tick();

    expect(screen.getByText('report⟨U+202E⟩gpj')).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('\u202E');
  });

  it('a batch that arrives after the first render fills the rows in place', async () => {
    const { backend } = await huge();
    viewport().scrollTop = 5_000 * 24;
    await fireEvent.scroll(viewport());
    expect(screen.queryByText('late')).not.toBeInTheDocument();

    backend.last().emit({ kind: 'batch', offset: 4_999, entries: [entry('late.txt')] }); // row 5 000 after `..`
    await tick();

    expect(screen.getByText('late')).toBeInTheDocument();
  });

  it('scrolling far down renders the rows there', async () => {
    await huge();

    viewport().scrollTop = 150 * 24;
    await fireEvent.scroll(viewport());

    expect(screen.getByText('f140')).toBeInTheDocument(); // row 150 = `..`, `sub`, then f0…
    expect(screen.queryByText('sub')).not.toBeInTheDocument();
  });

  it('double-clicking a folder opens it, `..` opens the parent, a crumb opens its folder', async () => {
    const { backend } = await huge();

    await fireEvent.dblClick(screen.getByText('sub'));
    expect(backend.last().args).toMatchObject({ path: '/big', child: 'sub' });

    await fireEvent.dblClick(screen.getByText('..'));
    expect(backend.last().args).toMatchObject({ path: '/', child: null });

    await fireEvent.click(screen.getByRole('button', { name: 'big' }));
    expect(backend.last().args).toMatchObject({ path: '/big', child: null });
  });

  it('shows the summary and the free space when done, and the error when the folder cannot be read', async () => {
    const { backend } = await huge();

    backend.last().emit({ kind: 'done', dirs: 1, files: 99_999, bytes: 2048, free: 5 * 1024 ** 3 });
    expect(await screen.findByText('1 folder · 99,999 files · 2.0 KB')).toBeInTheDocument();
    expect(screen.getByText('5.0 GB free')).toBeInTheDocument();

    backend.last().emit({ kind: 'error', error: { code: 'notFound', message: 'This folder does not exist.' } });
    expect(await screen.findByRole('alert')).toHaveTextContent('This folder does not exist.');
  });

  it('shows the backend`s error with the characters that reorder text written out', async () => {
    const { backend } = await huge();

    backend.last().emit({ kind: 'error', error: { code: 'notFound', message: 'No folder a\u202Etxt.exe' } });

    expect(await screen.findByRole('alert')).toHaveTextContent('No folder a⟨U+202E⟩txt.exe');
  });

  it('omits the free space when the backend cannot tell', async () => {
    const { backend } = await huge();

    backend.last().emit({ kind: 'done', dirs: 1, files: 2, bytes: 3, free: null });

    expect(await screen.findByText('1 folder · 2 files · 3 B')).toBeInTheDocument();
    expect(screen.queryByText(/free$/)).not.toBeInTheDocument();
  });
});

describe('Pane keyboard and accessibility', () => {
  it('the active pane`s list has the focus; the cursor is its active descendant', async () => {
    await huge();

    expect(list()).toHaveFocus();
    expect(list()).toHaveAttribute('aria-multiselectable', 'true');
    expect(list()).toHaveAttribute('aria-activedescendant', cursorRow().id);
    expect(cursorRow()).toHaveAttribute('role', 'option');
    expect(cursorRow()).toHaveTextContent('..');
  });

  it('an inactive pane does not take the focus; clicking it activates it and focuses its list', async () => {
    const { onActivate } = await huge(false);
    expect(list()).not.toHaveFocus();

    await fireEvent.click(screen.getByRole('region', { name: 'Left pane' }));

    expect(onActivate).toHaveBeenCalled();
    expect(list()).toHaveFocus();
  });

  it('arrow keys move the cursor and Page keys move by the visible rows', async () => {
    tenRowsTall();
    await huge();

    await fireEvent.keyDown(list(), { key: 'ArrowDown' });
    expect(cursorRow()).toHaveTextContent('sub');
    expect(list()).toHaveAttribute('aria-activedescendant', cursorRow().id);

    await fireEvent.keyDown(list(), { key: 'PageDown' });
    expect(cursorRow()).toHaveTextContent('f9'); // 10 rows of 24px fit: row 1 + 10 = f9
  });

  it('End scrolls the last row of a 100 000-entry folder into view', async () => {
    tenRowsTall();
    await huge();

    await fireEvent.keyDown(list(), { key: 'End' });
    await tick();

    expect(viewport().scrollTop).toBe((TOTAL + 1) * 24 - 240);
    expect(list()).toHaveAttribute('aria-activedescendant', cursorRow().id);
  });

  it('Space selects the entry: accent state, aria-selected and the selection summary', async () => {
    const { backend } = await huge();
    backend.last().emit({ kind: 'done', dirs: 1, files: 199, bytes: 1990, free: null });
    await fireEvent.keyDown(list(), { key: 'ArrowDown' });
    await fireEvent.keyDown(list(), { key: 'ArrowDown' }); // f0.txt

    await fireEvent.keyDown(list(), { key: ' ' });

    expect(screen.getByRole('option', { name: /f0/, selected: true })).toBeInTheDocument();
    expect(screen.getByText('Selected: 0 folders · 1 file · 10 B')).toBeInTheDocument();
  });

  it('Space on a folder shows the DIR tag until its size is counted, then the progress and the size', async () => {
    const { backend } = await huge();
    await fireEvent.keyDown(list(), { key: 'ArrowDown' });
    expect(screen.getByText('DIR')).toBeInTheDocument();

    await fireEvent.keyDown(list(), { key: ' ' });
    expect(backend.sizes[0].args).toMatchObject({ pane: 'left', path: '/big', name: 'sub' });

    backend.sizes[0].emit({ kind: 'progress', bytes: 1536 });
    expect(await screen.findByText('1.5 KB')).toBeInTheDocument();
    expect(screen.queryByText('DIR')).not.toBeInTheDocument();

    backend.sizes[0].emit({ kind: 'done', bytes: 3 * 1024 ** 2 });
    expect(await screen.findByText('3.0 MB')).toBeInTheDocument();
  });

  it('+ asks for a mask in the footer; Enter selects the matching files and returns the focus to the list', async () => {
    await huge();

    await fireEvent.keyDown(list(), { key: '+' });
    const input = await screen.findByRole('textbox', { name: /Select/ });
    expect(input).toHaveValue('*');
    expect(input).toHaveFocus();

    await fireEvent.input(input, { target: { value: 'f1?.txt' } });
    await fireEvent.keyDown(input, { key: 'Enter' });

    expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
    expect(list()).toHaveFocus();
    expect(await screen.findByText('Selected: 0 folders · 10 files · 100 B')).toBeInTheDocument();
  });

  it('Escape in the mask prompt cancels it', async () => {
    await huge();

    await fireEvent.keyDown(list(), { key: '-' });
    const input = await screen.findByRole('textbox', { name: /Deselect/ });
    await fireEvent.keyDown(input, { key: 'Escape' });

    expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
    expect(list()).toHaveFocus();
  });

  it('typing filters; the footer shows the badge with the match count', async () => {
    await huge();

    await fireEvent.keyDown(list(), { key: '1' });
    await fireEvent.keyDown(list(), { key: '9' });

    expect(await screen.findByText('Filter: 19 · 11')).toBeInTheDocument(); // f19, f119 and f190…f198
    expect(screen.getByText('..')).toBeInTheDocument();
    expect(screen.queryByText('sub')).not.toBeInTheDocument();

    await fireEvent.keyDown(list(), { key: 'Escape' });
    expect(screen.queryByText(/^Filter:/)).not.toBeInTheDocument();
  });

  it('a click puts the cursor on a row, Ctrl+click toggles it, Shift+click selects the range', async () => {
    await huge();

    await fireEvent.click(screen.getByText('f1'));
    expect(cursorRow()).toHaveTextContent('f1');

    await fireEvent.click(screen.getByText('f3'), { ctrlKey: true });
    expect(screen.getByRole('option', { name: /f3/, selected: true })).toBeInTheDocument();

    await fireEvent.click(screen.getByText('f5'), { shiftKey: true });
    expect(screen.getAllByRole('option', { selected: true })).toHaveLength(3); // f3, f4, f5
    expect(list()).toHaveFocus();
  });

  it('a double click with Ctrl, Cmd or Shift held does not open', async () => {
    const { backend } = await huge();

    await fireEvent.dblClick(screen.getByText('sub'), { ctrlKey: true });
    await fireEvent.dblClick(screen.getByText('sub'), { metaKey: true });
    await fireEvent.dblClick(screen.getByText('sub'), { shiftKey: true });

    expect(backend.opens).toHaveLength(1);
  });

  it('the column headers sort; the sorted one says so', async () => {
    await huge();
    const group = screen.getByRole('group', { name: 'Sort by' });
    expect(within(group).getByRole('button', { name: 'Name sorted ascending' })).toBeInTheDocument();
    expect(within(group).getByRole('button', { name: 'Size' })).toBeInTheDocument();

    await fireEvent.click(within(group).getByRole('button', { name: 'Size' }));
    expect(within(group).getByRole('button', { name: 'Size sorted ascending' })).toBeInTheDocument();
    expect(within(group).getByRole('button', { name: 'Name' })).toBeInTheDocument();

    await fireEvent.click(within(group).getByRole('button', { name: 'Size sorted ascending' }));
    expect(within(group).getByRole('button', { name: 'Size sorted descending' })).toBeInTheDocument();
  });

  it('shows the menu it is told to, under the header, and a click inside it leaves the focus in the menu', async () => {
    await huge(true, 'hotlist');

    const field = screen.getByRole('combobox', { name: 'Filter favourite folders' });
    expect(field).toHaveFocus();

    await fireEvent.click(field.closest('[data-menu]') as HTMLElement);
    expect(field).toHaveFocus();
  });

  it('a pane that is active with a menu open leaves the focus to the menu', async () => {
    await huge(true, 'drives');

    expect(screen.getByRole('listbox', { name: 'Left pane files' })).not.toHaveFocus();
    expect(screen.getByRole('listbox', { name: 'Drives' })).toHaveFocus();
  });

  it('the drive icon asks to show the drive menu, or to close it when it is shown', async () => {
    const closed = await huge();
    await fireEvent.click(screen.getByRole('button', { name: 'Drives' }));
    expect(closed.onMenu).toHaveBeenLastCalledWith('drives');
    cleanup();

    const shown = await huge(true, 'drives');
    await fireEvent.click(screen.getByRole('button', { name: 'Drives' }));
    expect(shown.onMenu).toHaveBeenLastCalledWith(undefined);
  });

  it('lays the header and the rows out on the columns the settings show, the narrow layout included', async () => {
    await huge();
    const pane = screen.getByRole('region', { name: 'Left pane' });
    const grid = (name: string) => pane.style.getPropertyValue(name);
    expect(grid('--cols-full')).toBe('20px minmax(0, 1fr) 48px 74px 118px');

    preferences.set({ showExt: false, showSize: false });
    await tick();

    expect(grid('--cols-full')).toBe('20px minmax(0, 1fr) 118px');
    expect(grid('--cols-narrow')).toBe('20px minmax(0, 1fr)');
    expect(grid('--rename-end')).toBe('3');
    expect(document.querySelector('.row .ex')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Size' })).not.toBeInTheDocument();
  });

  it('writes the extension after the name when its column is hidden, so that two files never read as one', async () => {
    const { backend } = await huge();
    backend.last().emit({ kind: 'batch', offset: 0, entries: [entry('report.pdf', 'file'), entry('report.png', 'file')] });
    await tick();
    const names = () => [...document.querySelectorAll('.row .nm')].slice(1, 3).map((name) => name.textContent);
    const inline = () => document.querySelector('.row .nx');

    expect(inline()).toHaveAttribute('aria-hidden', 'true'); // the Ext column is the one read out

    preferences.set({ showExt: false });
    await tick();

    expect(names()).toEqual(['report.pdf', 'report.png']);
    expect(inline()).toHaveClass('shown');
    expect(inline()).toHaveAttribute('aria-hidden', 'false');
  });
});
