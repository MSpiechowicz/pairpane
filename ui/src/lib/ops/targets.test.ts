import { describe, expect, it } from 'vitest';

import { PaneView } from '$lib/panes/view.svelte';

import { deliver, entry, fakeBackend } from '../../../tests/panes';
import { chosen, isFirstPart, requestFor, stemLength, targets } from './targets';

/** A pane that has listed `path` (rows: `..`, docs, a.txt, b.md, c.txt). */
async function shown(path = '/work') {
  const backend = fakeBackend();
  const view = new PaneView(backend.ipc, 'left');
  await deliver(backend, () => view.listing.open(path), path, [
    entry('docs', 'dir'),
    entry('a.txt', 'file'),
    entry('b.md', 'file'),
    entry('c.txt', 'file'),
  ]);
  return view;
}

describe('targets', () => {
  it('is the selection, in the folder`s order, even when the cursor is elsewhere', async () => {
    const view = await shown();
    view.setCursor(2);
    view.pick(4, 'toggle'); // c.txt (the cursor moves to it)
    view.pick(2, 'toggle'); // a.txt

    expect(targets(view)).toEqual(['a.txt', 'c.txt']);
  });

  it('leaves out the selected entries the quick filter hides', async () => {
    const view = await shown();
    view.selectAll();
    view.setFilter('a.');

    expect(targets(view)).toEqual(['a.txt']);

    view.setFilter('.txt');
    expect(targets(view)).toEqual(['a.txt', 'c.txt']);

    view.setFilter('');
    expect(targets(view)).toEqual(['docs', 'a.txt', 'b.md', 'c.txt']);
  });

  it('is the entry under the cursor when every selected entry is hidden', async () => {
    const view = await shown();
    view.pick(3, 'toggle'); // b.md
    view.setFilter('c.');

    expect(view.cursor).toBe(1);
    expect(targets(view)).toEqual(['c.txt']);
  });

  it('is the entry under the cursor when nothing is selected', async () => {
    const view = await shown();
    view.setCursor(3);

    expect(targets(view)).toEqual(['b.md']);
  });

  it('never counts the `..` row', async () => {
    const view = await shown();

    expect(view.isUp(view.cursor)).toBe(true);
    expect(targets(view)).toEqual([]);
  });

  it('is empty in a folder with no entries', async () => {
    const backend = fakeBackend();
    const view = new PaneView(backend.ipc, 'left');
    await deliver(backend, () => view.listing.open('/'), '/', []);

    expect(targets(view)).toEqual([]);
  });
});

describe('chosen', () => {
  it('is the entries targets names, in the folder`s order', async () => {
    const view = await shown();
    view.pick(4, 'toggle');
    view.pick(2, 'toggle');

    expect(chosen(view).map((item) => item.name)).toEqual(['a.txt', 'c.txt']);
    expect(chosen(view)[0]).toMatchObject({ name: 'a.txt', kind: 'file' });
  });

  it('is the entry under the cursor when nothing is selected, and nothing on `..`', async () => {
    const view = await shown();
    expect(chosen(view)).toEqual([]);

    view.setCursor(1);
    expect(chosen(view).map((item) => item.name)).toEqual(['docs']);
  });
});

describe('isFirstPart', () => {
  it('is a name that ends in .001', () => {
    expect(isFirstPart('movie.mkv.001')).toBe(true);
    expect(isFirstPart('a.001')).toBe(true);
    expect(isFirstPart('movie.mkv.002')).toBe(false);
    expect(isFirstPart('movie.mkv.0010')).toBe(false);
    expect(isFirstPart('001')).toBe(false);
  });
});

describe('requestFor', () => {
  it('combines the first part under the cursor into the other pane`s folder', async () => {
    const backend = fakeBackend();
    const source = new PaneView(backend.ipc, 'left');
    await deliver(backend, () => source.listing.open('/work'), '/work', [
      entry('movie.mkv.001', 'file'),
      entry('movie.mkv.002', 'file'),
      entry('old.001', 'dir'),
    ]);
    const other = await shown('/restore');

    source.setCursor(1);
    expect(requestFor('combine', source, other)).toEqual({
      kind: 'combine',
      from: '/work',
      name: 'movie.mkv.001',
      to: '/restore',
    });

    source.setCursor(2);
    expect(requestFor('combine', source, other)).toBeUndefined();
    source.setCursor(3); // a folder
    expect(requestFor('combine', source, other)).toBeUndefined();
    source.setCursor(0);
    expect(requestFor('combine', source, other)).toBeUndefined();
  });

  it('offers the other pane`s folder as the target of a copy or move', async () => {
    const source = await shown('/work');
    const other = await shown('/backup');
    source.setCursor(2);

    expect(requestFor('copy', source, other)).toEqual({ kind: 'copy', from: '/work', names: ['a.txt'], to: '/backup' });
    expect(requestFor('move', source, other)).toMatchObject({ kind: 'move' });
  });

  it('asks about the items on screen only, so the dialog count matches the selection shown', async () => {
    const source = await shown('/work');
    const other = await shown('/backup');
    source.selectAll();
    source.setFilter('a.');

    expect(requestFor('move', source, other)).toMatchObject({ names: ['a.txt'] });
  });

  it('is nothing when there is nothing to act on', async () => {
    const source = await shown();
    const other = await shown('/backup');

    expect(requestFor('copy', source, other)).toBeUndefined();
    expect(requestFor('rename', source, other)).toBeUndefined();
  });

  it('is nothing when the pane has not listed a folder', () => {
    const view = new PaneView(fakeBackend().ipc, 'left');

    expect(requestFor('copy', view, view)).toBeUndefined();
  });

  it('renames the entry under the cursor, whatever is selected', async () => {
    const source = await shown('/work');
    source.pick(3, 'toggle'); // b.md is selected and under the cursor
    source.pick(1, 'cursor');

    expect(requestFor('rename', source, source)).toEqual({ kind: 'rename', path: '/work', name: 'docs', dir: true });
  });

  it('makes a folder or a file in the pane`s own folder, even with nothing under the cursor', async () => {
    const source = await shown('/work'); // the cursor is on `..`
    const other = await shown('/backup');

    expect(requestFor('mkdir', source, other)).toEqual({ kind: 'mkdir', path: '/work' });
    expect(requestFor('mkfile', source, other)).toEqual({ kind: 'mkfile', path: '/work' });
    expect(requestFor('mkdir', new PaneView(fakeBackend().ipc, 'left'), other)).toBeUndefined();
  });

  it('links the entry under the cursor, whatever is selected, from the other pane`s folder', async () => {
    const source = await shown('/work');
    const other = await shown('/backup');
    source.pick(3, 'toggle');
    source.pick(2, 'cursor');

    expect(requestFor('symlink', source, other)).toEqual({
      kind: 'symlink',
      path: '/work',
      name: 'a.txt',
      dir: false,
      to: '/backup',
    });
    expect(requestFor('hardlink', source, other)).toMatchObject({ kind: 'hardlink', name: 'a.txt' });
  });

  it('links nothing from the `..` row, or into a pane that has not listed a folder', async () => {
    const source = await shown('/work');
    const empty = new PaneView(fakeBackend().ipc, 'right');

    expect(requestFor('symlink', source, source)).toBeUndefined();
    source.setCursor(2);
    expect(requestFor('symlink', source, empty)).toBeUndefined();
  });
});

describe('stemLength', () => {
  it('leaves the extension out, but not for a folder, a dot file or a trailing dot', () => {
    expect(stemLength('report.final.pdf', false)).toBe('report.final'.length);
    expect(stemLength('notes', false)).toBe(5);
    expect(stemLength('.bashrc', false)).toBe(7);
    expect(stemLength('trailing.', false)).toBe(9);
    expect(stemLength('v1.2', true)).toBe(4);
  });
});
