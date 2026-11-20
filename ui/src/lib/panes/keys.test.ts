import { describe, expect, it } from 'vitest';

import { deliver, entry, fakeBackend } from '../../../tests/panes';
import { handleKey } from './keys';
import { PaneView } from './view.svelte';

/** A pane showing `/work` (rows: `..`, docs, src, a.txt, b.md, c.TXT) and a list element to press keys on. */
async function shown(mac = false) {
  const backend = fakeBackend();
  const view = new PaneView(backend.ipc, 'left');
  await deliver(backend, () => view.listing.open('/work'), '/work', [
    entry('docs', 'dir'),
    entry('src', 'dir'),
    entry('a.txt', 'file', { size: 30 }),
    entry('b.md', 'file', { size: 10 }),
    entry('c.TXT', 'file', { size: 20 }),
  ]);

  const list = document.createElement('div');
  const child = list.appendChild(document.createElement('input'));
  let result = false;
  list.addEventListener('keydown', (event) => (result = handleKey(view, event, 3, mac)));

  const press = (key: string, init: KeyboardEventInit = {}, target: Element = list) => {
    result = false;
    target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init }));
    return result;
  };

  return { backend, view, press, child };
}

describe('handleKey', () => {
  it('arrows move by one, Page keys by the visible rows, Home and End to the ends', async () => {
    const { view, press } = await shown();

    expect(press('ArrowDown')).toBe(true);
    expect(view.cursor).toBe(1);
    press('PageDown');
    expect(view.cursor).toBe(4);
    press('End');
    expect(view.cursor).toBe(5);
    press('PageUp');
    expect(view.cursor).toBe(2);
    press('ArrowUp');
    expect(view.cursor).toBe(1);
    press('Home');
    expect(view.cursor).toBe(0);
  });

  it('Enter opens a folder or `..`, and leaves a file alone', async () => {
    const { backend, view, press } = await shown();

    view.setCursor(3);
    expect(press('Enter')).toBe(false);
    expect(backend.opens).toHaveLength(1);

    view.setCursor(1);
    expect(press('Enter')).toBe(true);
    expect(backend.last().args).toMatchObject({ path: '/work', child: 'docs' });

    view.setCursor(0);
    press('Enter');
    expect(backend.last().args).toMatchObject({ path: '/', child: null });
  });

  it('Space and Insert select and move down, `+`/`-` open the mask prompt, `*` inverts, Ctrl+A selects all', async () => {
    const { view, press } = await shown();
    view.setCursor(3);

    press(' ');
    press('Insert');
    expect(view.selection).toEqual({ dirs: 0, files: 2, bytes: 40 });

    press('+');
    expect(view.maskMode).toBe('select');
    press('-');
    expect(view.maskMode).toBe('deselect');

    press('*');
    expect(view.selection).toEqual({ dirs: 0, files: 1, bytes: 20 });

    press('a', { ctrlKey: true });
    expect(view.selection?.dirs).toBe(2);
    view.setCursor(0);
    press('a', { metaKey: true });
    expect(view.selection).toEqual({ dirs: 2, files: 3, bytes: 60 });
  });

  it('printable keys build the filter; Backspace trims it, then goes up; Escape clears it', async () => {
    const { backend, view, press } = await shown();

    press('t');
    press('x');
    expect(view.filter).toBe('tx');
    expect(view.matches).toBe(2);

    press('Backspace');
    expect(view.filter).toBe('t');
    press('Escape');
    expect(view.filter).toBe('');
    expect(press('Escape')).toBe(false);

    expect(press('Backspace')).toBe(true);
    expect(backend.last().args).toMatchObject({ path: '/', child: null });
  });

  it('AltGr characters (Ctrl+Alt) are typed, Ctrl and Cmd shortcuts are not', async () => {
    const { view, press } = await shown();

    expect(press('ą', { ctrlKey: true, altKey: true })).toBe(true);
    expect(view.filter).toBe('ą');
    view.setFilter('');

    expect(press('c', { ctrlKey: true })).toBe(false);
    expect(press('v', { metaKey: true })).toBe(false);
    expect(press('ArrowDown', { altKey: true })).toBe(false);
    expect(view.filter).toBe('');
  });

  it('Alt+letter is a menu accelerator, not a filter key, except as Option on macOS', async () => {
    const { view, press } = await shown();

    expect(press('f', { altKey: true })).toBe(false);
    expect(view.filter).toBe('');

    const mac = await shown(true);
    expect(mac.press('ƒ', { altKey: true })).toBe(true);
    expect(mac.view.filter).toBe('ƒ');
  });

  it('typing a name and pressing Enter opens the first match, even from the `..` row', async () => {
    const { backend, view, press } = await shown();
    expect(view.cursor).toBe(0);

    press('s');
    press('r');
    expect(view.entryAt(view.cursor)?.name).toBe('src');

    expect(press('Enter')).toBe(true);
    expect(backend.last().args).toMatchObject({ path: '/work', child: 'src' });
  });

  it('ignores Tab, composition and keys typed into something else', async () => {
    const { view, press, child } = await shown();

    expect(press('Tab')).toBe(false);
    expect(press('x', { isComposing: true })).toBe(false);
    expect(press('x', {}, child)).toBe(false);
    expect(view.filter).toBe('');
  });
});
