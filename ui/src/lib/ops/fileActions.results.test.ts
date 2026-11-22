import { describe, expect, it } from 'vitest';

import type { PaneId } from '$lib/ipc/bindings/PaneId';
import { IpcFailure } from '$lib/ipc/transport';
import { PaneTabs } from '$lib/panes/tabs.svelte';
import { Viewer } from '$lib/viewer/viewer.svelte';

import { arrive, entry, fakeBackend } from '../../../tests/panes';
import { type FileAction, perform } from './dialogs';
import { FileActions } from './fileActions.svelte';
import { INTO_RESULTS_NOTICE, RESULTS_NOTICE, RESULTS_TO_ARCHIVE_NOTICE } from './targets';

const ROWS = ['a/x.txt', 'a/y.txt', 'b/z.txt', 'top.txt'];

/** Lists the results of search 3 in `/work` in `tabs`: the rows are `ROWS`, below the folder searched. */
async function results(backend: ReturnType<typeof fakeBackend>, tabs: PaneTabs) {
  await tabs.view.listing.openResults(3);
  backend.last().emit({ kind: 'start', path: '/work', parent: '/', crumbs: [], total: ROWS.length, results: 3 });
  backend.last().emit({ kind: 'batch', offset: 0, entries: ROWS.map((name) => entry(name)) });
  backend.last().emit({ kind: 'done', dirs: 0, files: ROWS.length, bytes: 0, free: null });
}

/** The actions of a window whose left pane is active; `inside` says which pane shows the results, the other lists `/dest`. */
async function world(inside: PaneId, answers: Parameters<typeof fakeBackend>[0] = {}) {
  const backend = fakeBackend(answers);
  const panes = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
  const other = inside === 'left' ? panes.right : panes.left;

  await results(backend, panes[inside]);
  await other.view.listing.open('/dest');
  arrive(backend.last(), '/dest', []);

  const files = new FileActions(backend.ipc, panes, () => 'left');
  return { backend, panes, files, view: panes.left.view };
}

/** Selects the results whose rows are `rows` (row 0 is `..`). */
function select(view: PaneTabs['view'], ...rows: number[]) {
  rows.forEach((row) => view.pick(row, 'toggle'));
}

const sent = (backend: ReturnType<typeof fakeBackend>, command: string) =>
  backend.invoke.mock.calls.filter(([name]) => name === command).map(([, args]) => args);

describe('FileActions on search results', () => {
  it.each<FileAction>([
    'move',
    'rename',
    'renameInline',
    'newFolder',
    'newFile',
    'symlink',
    'hardlink',
    'properties',
    'attributes',
    'checksum',
    'verifyChecksum',
    'split',
    'combine',
    'extract',
  ])('%s is refused: the file’s folder has to be opened first, and nothing is opened or asked', async (action) => {
    const { backend, files, view } = await world('left');
    view.setCursor(1);
    const calls = backend.invoke.mock.calls.length;

    files.run(action);

    expect(files.toast).toMatchObject({ text: RESULTS_NOTICE, error: true });
    expect([files.dialog, files.removal, files.tools.dialog, files.rename.current]).toEqual([undefined, undefined, undefined, undefined]);
    expect(backend.invoke.mock.calls).toHaveLength(calls);
  });

  it('copy is never extract: the dialog is for the names below the folder searched, into the other pane’s folder', async () => {
    const { files, view } = await world('left');
    select(view, 2, 4);

    files.run('copy');

    expect(files.toast).toBeUndefined();
    expect(files.dialog?.request).toEqual({ kind: 'copy', from: '/work', names: ['a/y.txt', 'top.txt'], to: '/dest', results: true });
  });

  it('copy sends one job for each folder the results are in, in the order the folders appear', async () => {
    const { backend, files, view } = await world('left');
    select(view, 1, 2, 3, 4);
    files.run('copy');

    await perform(backend.ipc, files.dialog!.request, '/dest', true);

    expect(sent(backend, 'ops_start')).toEqual([
      { kind: 'copy', from: '/work/a', names: ['x.txt', 'y.txt'], to: '/dest', verify: true },
      { kind: 'copy', from: '/work/b', names: ['z.txt'], to: '/dest', verify: true },
      { kind: 'copy', from: '/work', names: ['top.txt'], to: '/dest', verify: true },
    ]);
  });

  it('starting the copy deselects the results', async () => {
    const { files, view } = await world('left');
    select(view, 1, 4);
    files.run('copy');

    files.dialogDone('/dest');

    expect(view.selectedShown()).toEqual([]);
  });

  it.each<FileAction>(['copy', 'move', 'symlink', 'split'])('%s into a pane that shows results is refused', async (action) => {
    const { files, view } = await world('right');
    view.setCursor(1);

    files.run(action);

    expect(files.toast).toMatchObject({ text: INTO_RESULTS_NOTICE, error: true });
    expect([files.dialog, files.tools.dialog]).toEqual([undefined, undefined]);
  });

  it('copy of results into a pane that shows an archive is refused', async () => {
    const { backend, files, panes, view } = await world('left');
    await panes.right.view.listing.openArchive({ archive: '/dest/a.zip', dir: '' });
    backend.last().emit({ kind: 'start', path: '/dest', parent: '/', crumbs: [], total: 0, archive: { archive: '/dest/a.zip', dir: '' } });
    view.setCursor(1);

    files.run('copy');

    expect(files.toast).toMatchObject({ text: RESULTS_TO_ARCHIVE_NOTICE, error: true });
    expect(files.dialog).toBeUndefined();
  });

  it('pack takes the names below the folder searched, from the folder searched, into the other pane’s folder', async () => {
    const { files, view } = await world('left');
    select(view, 1, 3);

    files.run('pack');

    expect(files.toast).toBeUndefined();
    expect(files.tools.dialog).toMatchObject({ kind: 'pack', path: '/work', names: ['a/x.txt', 'b/z.txt'], to: '/dest' });
  });
});

describe('removing search results', () => {
  it('asks about the names below the folder searched', async () => {
    const { files, view } = await world('left');
    select(view, 3);

    files.run('trash');

    expect(files.removal).toMatchObject({ permanent: false, path: '/work', names: ['b/z.txt'], results: true });
  });

  it('trash of results in several folders is one request for each, without Undo, and lists the results again', async () => {
    let token = 0;
    const { backend, files, view } = await world('left', { ops_trash: () => ({ undo: (token += 1), failure: null }) });
    select(view, 1, 2, 3, 4);
    files.run('trash');
    const opens = backend.opens.length;

    await files.confirmRemoval();

    expect(sent(backend, 'ops_trash')).toEqual([
      { path: '/work/a', names: ['x.txt', 'y.txt'] },
      { path: '/work/b', names: ['z.txt'] },
      { path: '/work', names: ['top.txt'] },
    ]);
    expect(files.toast).toMatchObject({ text: 'Moved 4 items to the Trash.', error: false, undo: null });
    expect(files.removal).toBeUndefined();
    expect(view.selectedShown()).toEqual([]);
    expect(backend.opens).toHaveLength(opens + 1);
    expect(backend.last().args).toMatchObject({ id: 3 });
  });

  it('trash of results of one folder offers Undo, and Undo restores that one request', async () => {
    const { backend, files, view } = await world('left', {
      ops_trash: () => ({ undo: 5, failure: null }),
      ops_untrash: () => 2,
    });
    select(view, 1, 2);
    files.run('trash');

    await files.confirmRemoval();

    expect(sent(backend, 'ops_trash')).toEqual([{ path: '/work/a', names: ['x.txt', 'y.txt'] }]);
    expect(files.toast).toMatchObject({ text: 'Moved 2 items to the Trash.', undo: 5 });

    await files.undo();

    expect(sent(backend, 'ops_untrash')).toEqual([{ token: 5 }]);
  });

  it('a later folder that cannot be trashed is told with what moved; Undo is offered for the one request that went', async () => {
    let calls = 0;
    const { backend, files, view } = await world('left', {
      ops_trash: () => {
        calls += 1;
        return calls === 1 ? { undo: 8, failure: null } : { undo: null, failure: { code: 'permissionDenied', message: 'Denied.' } };
      },
    });
    select(view, 1, 3, 4);
    files.run('trash');

    await files.confirmRemoval();

    expect(sent(backend, 'ops_trash')).toHaveLength(2); // the third folder is left alone
    expect(files.toast).toMatchObject({ text: 'Some items were moved to the Trash, but not all. Denied.', error: true, undo: 8 });
    expect(view.selectedShown().map((item) => item.name)).toEqual(['b/z.txt', 'top.txt']);
  });

  it('a refusal of the first folder rejects, so that the dialog stays', async () => {
    const refusal = new IpcFailure({ code: 'invalidInput', message: 'Not allowed.' });
    const { files, view } = await world('left', {
      ops_trash: () => Promise.reject(refusal),
    });
    select(view, 1);
    files.run('trash');

    await expect(files.confirmRemoval()).rejects.toBe(refusal);

    expect(files.removal).toBeDefined();
    expect(files.toast).toBeUndefined();
  });

  it('delete of results is one request for each folder; one refused leaves the dialog asking about the rest', async () => {
    const refusal = new IpcFailure({ code: 'invalidInput', message: 'Not allowed.' });
    let calls = 0;
    const { backend, files, view } = await world('left', {
      ops_delete: () => (++calls === 2 ? Promise.reject(refusal) : null),
    });
    select(view, 1, 2, 3, 4);
    files.run('deletePermanently');

    await expect(files.confirmRemoval()).rejects.toBe(refusal);

    expect(sent(backend, 'ops_delete')).toEqual([
      { path: '/work/a', names: ['x.txt', 'y.txt'] },
      { path: '/work/b', names: ['z.txt'] },
    ]);
    expect(files.removal?.names).toEqual(['b/z.txt', 'top.txt']);
    expect(view.selectedShown().map((item) => item.name)).toEqual(['b/z.txt', 'top.txt']);
  });
});

describe('Viewer on search results', () => {
  it('F3 opens the file from the folder that holds it', async () => {
    const { backend, panes } = await world('left');
    panes.left.view.setCursor(3); // b/z.txt
    const viewer = new Viewer(backend.ipc, panes, () => 'left');

    expect(viewer.under).toEqual({ path: '/work/b', name: 'z.txt' });
    viewer.open();

    expect(backend.invoke).toHaveBeenCalledWith('view_open', { path: '/work/b', name: 'z.txt' });
    viewer.close();
  });
});
