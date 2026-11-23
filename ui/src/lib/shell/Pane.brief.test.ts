import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { Hotlist } from '$lib/panes/hotlist.svelte';
import { PaneTabs } from '$lib/panes/tabs.svelte';
import { Volumes } from '$lib/panes/volumes.svelte';

import { entry, fakeBackend } from '../../../tests/panes';
import Pane from './Pane.svelte';

const TOTAL = 100_000;
const ROWS = 10;
const COLUMN = 200;

/** A pane in the brief layout over a folder of 100 000 entries: `sub`, then `f0.txt`, `f1.txt`… (only 200 have arrived). */
async function brief() {
  const backend = fakeBackend();
  const tabs = new PaneTabs(backend.ipc, 'left');
  await tabs.view.listing.open('/big');
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
  tabs.view.layout = 'brief';

  render(Pane, {
    label: 'Left pane',
    tabs,
    volumes: new Volumes(backend.ipc),
    hotlist: new Hotlist(),
    menu: undefined,
    active: true,
    onActivate: vi.fn(),
    onMenu: vi.fn(),
  });
  await tick();

  return { backend, tabs, view: tabs.view };
}

afterEach(() => vi.restoreAllMocks());

/** A list that holds 10 rows of 24px and 3 columns of 200px (the test DOM measures nothing). */
function sized() {
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(ROWS * 24);
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(3 * COLUMN);
}

const list = () => screen.getByRole('listbox');
const viewport = () => document.querySelector('.viewport') as HTMLElement;
const cursorRow = () => document.querySelector('.row.cursor') as HTMLElement;
const rowAt = (index: number) => document.querySelector(`[data-row="${index}"]`) as HTMLElement;
const renderedRows = () => [...document.querySelectorAll<HTMLElement>('.row')].map((row) => Number(row.dataset.row));

describe('Pane in the brief layout', () => {
  it('has no column header, and shows the icon and the name of each row with its extension, no other column', async () => {
    sized();
    await brief();

    expect(screen.queryByRole('group', { name: 'Sort by' })).not.toBeInTheDocument();
    expect(document.querySelector('.row .ex, .row .sz, .row .dt')).toBeNull();
    expect(rowAt(0)).toHaveTextContent('..');
    expect(rowAt(1)).toHaveTextContent('sub');
    expect(rowAt(2)).toHaveTextContent('f0.txt');
    expect(rowAt(2).querySelector('.nx')).toHaveClass('shown');
  });

  it('lists the items column by column: a column holds as many as fit the height, the next one continues', async () => {
    sized();
    await brief();

    const window = document.querySelector('.window') as HTMLElement;
    expect(window.style.gridTemplateRows).toBe(`repeat(${ROWS}, 24px)`);
    expect(window.style.getPropertyValue('grid-auto-columns')).toBe(`${COLUMN}px`);

    const rows = renderedRows();
    expect(rows.slice(0, 12)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]); // in order: the grid fills a column first
    expect(rowAt(9)).toHaveTextContent('f7.txt'); // the last of the first column
    expect(rowAt(10)).toHaveTextContent('f8.txt'); // the top of the second
    expect((document.querySelector('.spacer') as HTMLElement).style.width).toBe(`${Math.ceil((TOTAL + 1) / ROWS) * COLUMN}px`);
  });

  it('renders only the columns in view of a 100 000-entry folder, and the ones scrolled to', async () => {
    sized();
    await brief();
    expect(renderedRows().length).toBeLessThan(60);
    expect(screen.queryByText('f48')).not.toBeInTheDocument(); // the sixth column

    viewport().scrollLeft = 5 * COLUMN;
    await fireEvent.scroll(viewport());

    expect(rowAt(50)).toHaveTextContent('f48.txt');
    expect(screen.queryByText('..')).not.toBeInTheDocument();
    expect(renderedRows().length).toBeLessThan(80);
    expect((document.querySelector('.window') as HTMLElement).style.transform).toBe(`translateX(${3 * COLUMN}px)`);
  });

  it('keeps the cursor in view: sideways to the column it moves to', async () => {
    sized();
    const { view } = await brief();

    await fireEvent.keyDown(list(), { key: 'End' });
    await tick();

    expect(view.cursor).toBe(TOTAL);
    expect(viewport().scrollLeft).toBe(Math.ceil((TOTAL + 1) / ROWS) * COLUMN - 3 * COLUMN);
    expect(list()).toHaveAttribute('aria-activedescendant', cursorRow().id);

    await fireEvent.keyDown(list(), { key: 'Home' });
    await tick();
    expect(viewport().scrollLeft).toBe(0);
  });

  it('Left and Right move the cursor by a column, the Page keys too, Up and Down by one', async () => {
    sized();
    const { view } = await brief();

    await fireEvent.keyDown(list(), { key: 'ArrowRight' });
    expect(view.cursor).toBe(ROWS);
    expect(cursorRow()).toHaveTextContent('f8.txt');

    await fireEvent.keyDown(list(), { key: 'ArrowDown' });
    expect(view.cursor).toBe(ROWS + 1);

    await fireEvent.keyDown(list(), { key: 'PageDown' });
    expect(view.cursor).toBe(2 * ROWS + 1);

    await fireEvent.keyDown(list(), { key: 'ArrowLeft' });
    await fireEvent.keyDown(list(), { key: 'PageUp' });
    expect(view.cursor).toBe(1);

    await fireEvent.keyDown(list(), { key: 'ArrowUp' });
    expect(view.cursor).toBe(0);
  });

  it('a click, Ctrl+click and Shift+click select and a double click opens, as in the detailed layout', async () => {
    sized();
    const { backend, view } = await brief();

    await fireEvent.click(screen.getByText('f1'));
    expect(cursorRow()).toHaveTextContent('f1.txt');

    await fireEvent.click(screen.getByText('f3'), { ctrlKey: true });
    await fireEvent.click(screen.getByText('f5'), { shiftKey: true });
    expect(screen.getAllByRole('option', { selected: true })).toHaveLength(3); // f3, f4, f5
    expect(view.selectedShown().map((selected) => selected.name)).toEqual(['f3.txt', 'f4.txt', 'f5.txt']);
    expect(list()).toHaveFocus();

    await fireEvent.dblClick(screen.getByText('sub'));
    expect(backend.last().args).toMatchObject({ path: '/big', child: 'sub' });
    await fireEvent.dblClick(screen.getByText('..'));
    expect(backend.last().args).toMatchObject({ path: '/', child: null });
  });

  it('shows the whole name of an item in its tooltip, as its column may cut it short', async () => {
    sized();
    await brief();

    expect(rowAt(2).querySelector('.nm')).toHaveAttribute('title', 'f0.txt');
  });

  it('going back to the detailed layout brings the header and the columns back', async () => {
    sized();
    const { view } = await brief();

    view.layout = 'detailed';
    await tick();

    expect(screen.getByRole('group', { name: 'Sort by' })).toBeInTheDocument();
    expect(document.querySelector('.row.brief')).toBeNull();
    expect(document.querySelector('.row .sz')).not.toBeNull();
    expect(document.querySelector('.window')?.getAttribute('style') ?? '').not.toContain('grid-template-rows');
  });
});
