import { describe, expect, it } from 'vitest';

import type { Crumb } from '$lib/ipc/bindings/Crumb';
import type { Entry } from '$lib/ipc/bindings/Entry';

import { arrive, deliver, entry, fakeBackend } from '../../../tests/panes';
import { handleKey } from './keys';
import { PaneTabs } from './tabs.svelte';
import { type ArchiveSpot, joinInner } from './archive';
import { PaneView } from './view.svelte';

type Backend = ReturnType<typeof fakeBackend>;

/** Lets the last open deliver the folder `spot.dir` of an archive in `/work`, crumbs as the backend sends them. */
function arriveIn(backend: Backend, spot: ArchiveSpot, entries: Entry[] = []) {
  const name = spot.archive.split('/').pop() ?? '';
  const crumbs: Crumb[] = [
    { name: '/', path: '/' },
    { name: 'work', path: '/work' },
    { name: name, path: spot.archive, inside: '' },
  ];
  let inside = '';
  for (const part of spot.dir.split('/').filter((part) => part !== '')) {
    inside = inside === '' ? part : `${inside}/${part}`;
    crumbs.push({ name: part, path: spot.archive, inside });
  }

  const channel = backend.last();
  channel.emit({ kind: 'start', path: '/work', parent: '/', crumbs, total: entries.length, archive: spot });
  channel.emit({ kind: 'batch', offset: 0, entries });
  channel.emit({ kind: 'done', dirs: 0, files: entries.length, bytes: 0, free: null });
}

/** A pane showing `/work` with a folder, an archive and a plain file. */
async function shown() {
  const backend = fakeBackend();
  const view = new PaneView(backend.ipc, 'left');
  await deliver(backend, () => view.listing.open('/work'), '/work', [
    entry('docs', 'dir'),
    entry('a.zip', 'file'),
    entry('b.txt', 'file'),
  ]);
  return { backend, view };
}

const rowOf = (view: PaneView, name: string) =>
  Array.from({ length: view.count }, (_, row) => row).find((row) => view.entryAt(row)?.name === name) ?? -1;

const ZIP: ArchiveSpot = { archive: '/work/a.zip', dir: '' };

/** A pane inside `/work/a.zip` (rows: `..`, docs, inner.zip, readme). */
async function inside(dir = '') {
  const { backend, view } = await shown();
  await view.open(rowOf(view, 'a.zip'));
  arriveIn(backend, ZIP, [entry('docs', 'dir'), entry('inner.zip', 'file'), entry('readme', 'file')]);
  let here = '';
  for (const part of dir.split('/').filter((part) => part !== '')) {
    await view.open(rowOf(view, part));
    here = joinInner(here, part);
    arriveIn(backend, { archive: ZIP.archive, dir: here }, [entry('sub', 'dir')]);
  }
  return { backend, view };
}

describe('opening an archive', () => {
  it('lists the root of an archive file the cursor opens, and shows it read-only without a path', async () => {
    const { backend, view } = await shown();
    view.setCursor(rowOf(view, 'a.zip'));

    await view.open(view.cursor);
    expect(backend.invoke).toHaveBeenLastCalledWith(
      'archive_open',
      expect.objectContaining({ pane: 'left', archive: '/work/a.zip', dir: '', showHidden: false }),
      { signal: expect.any(AbortSignal) },
    );

    arriveIn(backend, ZIP, [entry('readme', 'file')]);
    const { listing } = view;
    expect(listing.archive).toEqual(ZIP);
    expect(listing.readonly).toBe(true);
    expect(listing.path).toBeUndefined();
    expect(listing.folder).toBe('/work');
    expect(listing.crumbs.map((crumb) => [crumb.name, crumb.inside])).toEqual([
      ['/', undefined],
      ['work', undefined],
      ['a.zip', ''],
    ]);
    expect(listing.entries.map((e) => e?.name)).toEqual(['readme']);
    expect(view.up).toBe(true);
  });

  it('opens the archive with Enter, and Enter on a plain file does nothing', async () => {
    const { backend, view } = await shown();
    const list = document.createElement('div');
    let handled = false;
    list.addEventListener('keydown', (event) => (handled = handleKey(view, event, 3, false)));
    const press = () => list.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', cancelable: true }));

    view.setCursor(rowOf(view, 'b.txt'));
    press();
    expect(handled).toBe(false);
    expect(backend.opens).toHaveLength(1);

    view.setCursor(rowOf(view, 'a.zip'));
    press();
    expect(handled).toBe(true);
    expect(backend.last().args).toMatchObject({ archive: '/work/a.zip', dir: '' });
  });

  it('enters inner folders and lists them again on reload, keeping hidden entries as asked', async () => {
    const { backend, view } = await inside();

    await view.open(rowOf(view, 'docs'));
    expect(backend.last().args).toMatchObject({ archive: '/work/a.zip', dir: 'docs' });
    arriveIn(backend, { archive: ZIP.archive, dir: 'docs' }, [entry('sub', 'dir')]);

    await view.open(rowOf(view, 'sub'));
    expect(backend.last().args).toMatchObject({ dir: 'docs/sub' });
    arriveIn(backend, { archive: ZIP.archive, dir: 'docs/sub' });

    await view.listing.setShowHidden(true);
    expect(backend.last().args).toMatchObject({ archive: '/work/a.zip', dir: 'docs/sub', showHidden: true });
  });

  it('does not open an archive inside an archive', async () => {
    const { backend, view } = await inside();
    const before = backend.opens.length;

    expect(view.opens(rowOf(view, 'inner.zip'))).toBe(false);
    await view.open(rowOf(view, 'inner.zip'));

    expect(backend.opens).toHaveLength(before);
    expect(view.listing.archive).toEqual(ZIP);
  });

  it('shows a new place of the archive from the top, selection and filter forgotten', async () => {
    const { backend, view } = await inside();
    view.setCursor(rowOf(view, 'readme'));
    view.insert();
    expect(view.selectedShown()).toHaveLength(1);

    await view.open(rowOf(view, 'docs'));
    arriveIn(backend, { archive: ZIP.archive, dir: 'docs' }, [entry('sub', 'dir')]);

    expect(view.selectedShown()).toHaveLength(0);
    expect(view.listing.location).not.toBe(`${ZIP.archive}\0`);
  });
});

describe('leaving an archive', () => {
  it('goes up to the folder above in the archive with the cursor on the folder left', async () => {
    const { backend, view } = await inside('docs/sub');

    await view.goUp();
    expect(backend.last().args).toMatchObject({ archive: '/work/a.zip', dir: 'docs' });
    arriveIn(backend, { archive: ZIP.archive, dir: 'docs' }, [
      entry('a', 'dir'),
      entry('sub', 'dir'),
      entry('z', 'dir'),
    ]);

    expect(view.entryAt(view.cursor)?.name).toBe('sub');
  });

  it('goes up from the root to the folder that holds the archive with the cursor on it', async () => {
    const { backend, view } = await inside();

    await view.goUp();
    expect(backend.last().args).toEqual(expect.objectContaining({ pane: 'left', path: '/work', child: null }));
    expect(backend.last().args).not.toHaveProperty('archive');
    arrive(backend.last(), '/work', [entry('b.txt'), entry('a.zip'), entry('c.txt')]);

    expect(view.listing.archive).toBeUndefined();
    expect(view.listing.path).toBe('/work');
    expect(view.listing.readonly).toBe(false);
    expect(view.entryAt(view.cursor)?.name).toBe('a.zip');
  });

  it('goes up with Backspace and from the `..` row too', async () => {
    const { backend, view } = await inside('docs');
    const list = document.createElement('div');
    list.addEventListener('keydown', (event) => handleKey(view, event, 3, false));

    list.dispatchEvent(new KeyboardEvent('keydown', { key: 'Backspace', cancelable: true }));
    expect(backend.last().args).toMatchObject({ archive: '/work/a.zip', dir: '' });
    arriveIn(backend, ZIP, [entry('docs', 'dir')]);

    await view.open(0);
    expect(backend.last().args).toMatchObject({ path: '/work' });
  });

  it('opens an archive step of the path bar by its folder', async () => {
    const { backend, view } = await inside('docs/sub');
    const step = view.listing.crumbs.find((crumb) => crumb.inside === 'docs')!;

    await view.listing.openArchive({ archive: step.path, dir: step.inside! });

    expect(backend.last().args).toMatchObject({ archive: '/work/a.zip', dir: 'docs' });
  });
});

describe('an archive listing in flight', () => {
  it('drops the events of an open that another one replaced', async () => {
    const { backend, view } = await shown();

    await view.listing.openArchive(ZIP);
    const earlier = backend.last();
    await view.listing.open('/work');
    arriveIn({ ...backend, last: () => earlier }, ZIP, [entry('late')]);

    expect(view.listing.archive).toBeUndefined();
    expect(view.listing.entries.map((e) => e?.name)).toEqual(['docs', 'a.zip', 'b.txt']);
  });

  it('stays in the folder with a notice when the archive cannot be opened', async () => {
    const backend = fakeBackend({
      archive_open: () => Promise.reject({ code: 'unsupported', message: 'Install unrar to open RAR archives' }),
    });
    const view = new PaneView(backend.ipc, 'left');
    await deliver(backend, () => view.listing.open('/work'), '/work', [entry('a.rar')]);

    await view.open(rowOf(view, 'a.rar'));

    expect(view.listing.notice).toBe('Install unrar to open RAR archives');
    expect(view.listing.path).toBe('/work');
    expect(view.listing.status).toBe('ready');
  });
});

describe('tabs inside an archive', () => {
  async function tabsIn() {
    const backend = fakeBackend();
    const tabs = new PaneTabs(backend.ipc, 'left');
    await deliver(backend, () => tabs.view.listing.open('/work'), '/work', [entry('a.zip')]);
    await tabs.view.listing.openArchive({ archive: '/work/a.zip', dir: 'docs' });
    arriveIn(backend, { archive: '/work/a.zip', dir: 'docs' });
    return { backend, tabs };
  }

  it('saves the folder that holds the archive', async () => {
    const { tabs } = await tabsIn();

    expect(tabs.state.map((tab) => tab.path)).toEqual(['/work']);
  });

  it('shows that folder when the tab is left and come back to', async () => {
    const { backend, tabs } = await tabsIn();

    tabs.add();
    arrive(backend.last(), '/work');
    tabs.select(0);

    expect(backend.last().args).toMatchObject({ path: '/work' });
    expect(backend.last().args).not.toHaveProperty('archive');
    expect(tabs.state.map((tab) => tab.path)).toEqual(['/work', '/work']);
  });

  it('restores a saved tab as the folder', async () => {
    const backend = fakeBackend();
    const tabs = new PaneTabs(backend.ipc, 'left');

    tabs.restore([{ path: '/work', sort: 'name', desc: false }], 0);

    expect(backend.last().args).toMatchObject({ path: '/work' });
  });

  it('does not make a step of the history of going into or around an archive', async () => {
    const { tabs } = await tabsIn();

    expect(tabs.canBack).toBe(false);
  });
});
