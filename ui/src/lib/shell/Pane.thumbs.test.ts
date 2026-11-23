import { fireEvent, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { entry } from '../../../tests/panes';
import {
  ACROSS,
  asked,
  cursorTile,
  list,
  refusal,
  renderedTiles,
  ROWS,
  sized,
  thumbnails,
  tileAt,
  TILE_HEIGHT,
  TILE_WIDTH,
  TOTAL,
  viewport,
} from '../../../tests/thumbs';

afterEach(() => vi.restoreAllMocks());

describe('Pane in the thumbnails layout', () => {
  it('has no column header and shows tiles: the icon of what is not a picture, `..` too, and the name under it', async () => {
    sized();
    await thumbnails({ thumb_get: () => Promise.reject(refusal('unsupported')) });

    expect(screen.queryByRole('group', { name: 'Sort by' })).not.toBeInTheDocument();
    expect(document.querySelector('.row .ex, .row .sz, .row .dt')).toBeNull();
    expect(tileAt(0)).toHaveClass('tile');
    expect(tileAt(0).querySelector('.ic.up')).not.toBeNull();
    expect(tileAt(0)).toHaveTextContent('..');
    expect(tileAt(1).querySelector('.ic.dir')).not.toBeNull();
    expect(tileAt(1)).toHaveTextContent('sub');
    expect(tileAt(2)).toHaveTextContent('p0.jpg');
  });

  it('lays the items out in rows of as many tiles as fit the width, row after row', async () => {
    sized();
    await thumbnails({ thumb_get: () => Promise.reject(refusal('unsupported')) });

    const window = document.querySelector('.window') as HTMLElement;
    expect(window.style.gridTemplateColumns).toBe(`repeat(${ACROSS}, ${TILE_WIDTH}px)`);
    expect(window.style.getPropertyValue('grid-auto-rows')).toBe(`${TILE_HEIGHT}px`);
    expect(renderedTiles().slice(0, 12)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
    expect((document.querySelector('.spacer') as HTMLElement).style.height).toBe(`${Math.ceil((TOTAL + 1) / ACROSS) * TILE_HEIGHT}px`);
  });

  it('shares a width that is no whole number of tiles among the tiles of a row', async () => {
    vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(ROWS * TILE_HEIGHT);
    vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(2 * TILE_WIDTH + 70);
    await thumbnails({ thumb_get: () => Promise.reject(refusal('unsupported')) });

    const window = document.querySelector('.window') as HTMLElement;
    expect(window.style.gridTemplateColumns).toBe(`repeat(2, ${(2 * TILE_WIDTH + 70) / 2}px)`);
  });

  it('lays out nothing before the pane has a size, rather than one row per item', async () => {
    vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(0);
    vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(0);
    const { backend } = await thumbnails();

    expect((document.querySelector('.spacer') as HTMLElement).style.height).toBe('0px');
    expect(document.querySelectorAll('.row')).toHaveLength(0);
    expect(asked(backend)).toEqual([]);
  });

  it('renders only the rows in view of a 100 000-entry folder, and the ones scrolled to', async () => {
    sized();
    await thumbnails({ thumb_get: () => Promise.reject(refusal('unsupported')) });
    expect(renderedTiles().length).toBeLessThanOrEqual((ROWS + 1) * ACROSS);
    expect(screen.queryByText('p48')).not.toBeInTheDocument();

    viewport().scrollTop = 17 * TILE_HEIGHT;
    await fireEvent.scroll(viewport());

    expect(tileAt(17 * ACROSS)).toHaveTextContent('p49.jpg'); // 51 items in rows 0 to 16
    expect(screen.queryByText('..')).not.toBeInTheDocument();
    expect(renderedTiles().length).toBeLessThanOrEqual((ROWS + 2) * ACROSS);
    expect((document.querySelector('.window') as HTMLElement).style.transform).toBe(`translateY(${16 * TILE_HEIGHT}px)`);
  });

  it('cuts a long name and shows it whole in the tooltip', async () => {
    sized();
    const long = `${'a very long picture name '.repeat(4)}.jpg`;
    await thumbnails({ thumb_get: () => Promise.reject(refusal('unsupported')) }, [entry(long)]);

    expect(tileAt(1).querySelector('.nm')).toHaveAttribute('title', long);
  });

  it('shows names with the characters that mislead written out', async () => {
    sized();
    await thumbnails({ thumb_get: () => Promise.reject(refusal('unsupported')) }, [entry('gpj.‮gnp')]);

    expect(tileAt(1)).toHaveTextContent('⟨U+202E⟩');
    expect(tileAt(1).querySelector('.nm')).toHaveAttribute('title', 'gpj.⟨U+202E⟩gnp');
  });
});

describe('Pane keys in the thumbnails layout', () => {
  it('Left and Right move by one item, Up and Down by a row of tiles, the Page keys by the rows in view', async () => {
    sized();
    const { view } = await thumbnails({ thumb_get: () => Promise.reject(refusal('unsupported')) });

    await fireEvent.keyDown(list(), { key: 'ArrowRight' });
    expect(view.cursor).toBe(1);
    expect(cursorTile()).toHaveTextContent('sub');

    await fireEvent.keyDown(list(), { key: 'ArrowDown' });
    expect(view.cursor).toBe(1 + ACROSS);

    await fireEvent.keyDown(list(), { key: 'PageDown' });
    expect(view.cursor).toBe(1 + ACROSS + ROWS * ACROSS);

    await fireEvent.keyDown(list(), { key: 'ArrowLeft' });
    expect(view.cursor).toBe(ACROSS + ROWS * ACROSS);

    await fireEvent.keyDown(list(), { key: 'PageUp' });
    expect(view.cursor).toBe(ACROSS);

    await fireEvent.keyDown(list(), { key: 'ArrowUp' });
    expect(view.cursor).toBe(0);
  });

  it('Up in the first row and Down in the last stay where they are, and Down into a short last row goes to its last tile', async () => {
    sized();
    const { view } = await thumbnails({ thumb_get: () => Promise.reject(refusal('unsupported')) }, [
      entry('a.txt'),
      entry('b.txt'),
      entry('c.txt'),
      entry('d.txt'),
    ]); // `..`, a, b, c | d

    await fireEvent.keyDown(list(), { key: 'ArrowRight' });
    await fireEvent.keyDown(list(), { key: 'ArrowRight' });
    await fireEvent.keyDown(list(), { key: 'ArrowUp' });
    expect(view.cursor).toBe(2);

    await fireEvent.keyDown(list(), { key: 'ArrowDown' });
    expect(view.cursor).toBe(4);

    await fireEvent.keyDown(list(), { key: 'ArrowDown' });
    expect(view.cursor).toBe(4);

    await fireEvent.keyDown(list(), { key: 'ArrowLeft' });
    await fireEvent.keyDown(list(), { key: 'ArrowLeft' });
    await fireEvent.keyDown(list(), { key: 'ArrowLeft' });
    await fireEvent.keyDown(list(), { key: 'ArrowLeft' });
    expect(view.cursor).toBe(0);
    await fireEvent.keyDown(list(), { key: 'ArrowLeft' });
    expect(view.cursor).toBe(0);
  });

  it('Home and End go to the first and the last item, and the list scrolls to keep the cursor in view', async () => {
    sized();
    const { view } = await thumbnails({ thumb_get: () => Promise.reject(refusal('unsupported')) });

    await fireEvent.keyDown(list(), { key: 'End' });
    await tick();
    expect(view.cursor).toBe(TOTAL);
    expect(viewport().scrollTop).toBe(Math.ceil((TOTAL + 1) / ACROSS) * TILE_HEIGHT - ROWS * TILE_HEIGHT);
    expect(list()).toHaveAttribute('aria-activedescendant', cursorTile().id);

    await fireEvent.keyDown(list(), { key: 'Home' });
    await tick();
    expect(view.cursor).toBe(0);
    expect(viewport().scrollTop).toBe(0);
  });

  it('scrolls by the least that brings the cursor`s row into view as it moves down and up', async () => {
    sized();
    await thumbnails({ thumb_get: () => Promise.reject(refusal('unsupported')) });

    for (let step = 0; step < ROWS; step += 1) {
      await fireEvent.keyDown(list(), { key: 'ArrowDown' });
    }
    await tick();
    expect(viewport().scrollTop).toBe(TILE_HEIGHT); // the cursor is in the 4th row: one row of tiles scrolled away

    await fireEvent.keyDown(list(), { key: 'ArrowUp' });
    await tick();
    expect(viewport().scrollTop).toBe(TILE_HEIGHT);
    await fireEvent.keyDown(list(), { key: 'ArrowUp' });
    await fireEvent.keyDown(list(), { key: 'ArrowUp' });
    await tick();
    expect(viewport().scrollTop).toBe(0);
  });

  it('a click, Ctrl+click and Shift+click select and a double click opens, as in the detailed layout', async () => {
    sized();
    const { backend, view } = await thumbnails({ thumb_get: () => Promise.reject(refusal('unsupported')) });

    await fireEvent.click(screen.getByText('p1'));
    expect(cursorTile()).toHaveTextContent('p1.jpg');

    await fireEvent.click(screen.getByText('p3'), { ctrlKey: true });
    await fireEvent.click(screen.getByText('p5'), { shiftKey: true });
    expect(view.selectedShown().map((selected) => selected.name)).toEqual(['p3.jpg', 'p4.jpg', 'p5.jpg']);
    expect(screen.getAllByRole('option', { selected: true })).toHaveLength(3);
    expect(list()).toHaveFocus();

    await fireEvent.dblClick(screen.getByText('sub'));
    expect(backend.last().args).toMatchObject({ path: '/big', child: 'sub' });
    await fireEvent.dblClick(screen.getByText('..'));
    expect(backend.last().args).toMatchObject({ path: '/', child: null });
  });

  it('Space marks the item under the cursor and moves on, as in the other layouts', async () => {
    sized();
    const { view } = await thumbnails({ thumb_get: () => Promise.reject(refusal('unsupported')) });

    await fireEvent.keyDown(list(), { key: 'ArrowDown' }); // the first tile of the second row: p1.jpg
    await fireEvent.keyDown(list(), { key: ' ' });

    expect(view.selectedShown().map((selected) => selected.name)).toEqual(['p1.jpg']);
    expect(view.cursor).toBe(ACROSS + 1);
  });
});
