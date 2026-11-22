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

/** A folder and what it holds, as a search for `*` finds them (rows 1 to 3; the row 4 is another folder's file). */
const NESTED = ['d', 'd/f', 'd/sub/g', 'e/h'];

/** Lists the results of search 3 in `/work` in `tabs`: the rows are `rows`, below the folder searched. */
async function results(backend: ReturnType<typeof fakeBackend>, tabs: PaneTabs, rows = ROWS) {
  await tabs.view.listing.openResults(3);
  backend.last().emit({ kind: 'start', path: '/work', parent: '/', crumbs: [], total: rows.length, results: 3 });
  backend.last().emit({ kind: 'batch', offset: 0, entries: rows.map((name) => entry(name)) });
  backend.last().emit({ kind: 'done', dirs: 0, files: rows.length, bytes: 0, free: null });
}

/** The actions of a window whose left pane is active; `inside` says which pane shows the results, the other lists `/dest`. */
async function world(inside: PaneId, answers: Parameters<typeof fakeBackend>[0] = {}, rows = ROWS) {
  const backend = fakeBackend(answers);
  const panes = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
  const other = inside === 'left' ? panes.right : panes.left;

  await results(backend, panes[inside], rows);
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

  it('editing a result is refused until its folder is opened: the card says so, and nothing is opened', async () => {
    const { backend, files, view } = await world('left');
    view.setCursor(1);
    const calls = backend.invoke.mock.calls.length;

    expect(files.refuseEdit()).toBe(true);

    expect(files.toast).toMatchObject({ text: RESULTS_NOTICE, error: true });
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
      { kind: 'copy', from: '/work/a', names: ['x.txt', 'y.txt'], to: '/dest', verify: true, within: { root: '/work', dir: 'a' } },
      { kind: 'copy', from: '/work/b', names: ['z.txt'], to: '/dest', verify: true, within: { root: '/work', dir: 'b' } },
      { kind: 'copy', from: '/work', names: ['top.txt'], to: '/dest', verify: true, within: { root: '/work', dir: '' } },
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
      { path: '/work/a', names: ['x.txt', 'y.txt'], within: { root: '/work', dir: 'a' } },
      { path: '/work/b', names: ['z.txt'], within: { root: '/work', dir: 'b' } },
      { path: '/work', names: ['top.txt'], within: { root: '/work', dir: '' } },
    ]);
    expect(files.toast).toMatchObject({
      text: 'Moved 4 items to the Trash. Undo is only offered when one folder is affected.',
      error: false,
      undo: null,
    });
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

    expect(sent(backend, 'ops_trash')).toEqual([{ path: '/work/a', names: ['x.txt', 'y.txt'], within: { root: '/work', dir: 'a' } }]);
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
      { path: '/work/a', names: ['x.txt', 'y.txt'], within: { root: '/work', dir: 'a' } },
      { path: '/work/b', names: ['z.txt'], within: { root: '/work', dir: 'b' } },
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

describe('a folder and what it holds, both selected in search results', () => {
  it('copy is about the folder only: one job, the folder, and the dialog counts one item', async () => {
    const { backend, files, view } = await world('left', {}, NESTED);
    view.selectAll();

    files.run('copy');
    await perform(backend.ipc, files.dialog!.request, '/dest', true);

    expect(files.dialog?.request).toMatchObject({ names: ['d', 'e/h'] });
    expect(sent(backend, 'ops_start')).toEqual([
      { kind: 'copy', from: '/work', names: ['d'], to: '/dest', verify: true, within: { root: '/work', dir: '' } },
      { kind: 'copy', from: '/work/e', names: ['h'], to: '/dest', verify: true, within: { root: '/work', dir: 'e' } },
    ]);
  });

  it('starting the copy deselects what is inside the folder too', async () => {
    const { files, view } = await world('left', {}, NESTED);
    view.selectAll();
    files.run('copy');

    files.dialogDone('/dest');

    expect(view.selectedShown()).toEqual([]);
  });

  it.each([
    ['trash', 'ops_trash', { undo: 4, failure: null }],
    ['deletePermanently', 'ops_delete', null],
  ] as const)('%s asks about and sends the folder once, not what is inside it', async (action, command, answer) => {
    const { backend, files, view } = await world('left', { [command]: () => answer }, NESTED);
    view.selectAll();

    files.run(action);
    expect(files.removal?.names).toEqual(['d', 'e/h']);
    await files.confirmRemoval();

    expect(sent(backend, command)).toEqual([
      { path: '/work', names: ['d'], within: { root: '/work', dir: '' } },
      { path: '/work/e', names: ['h'], within: { root: '/work', dir: 'e' } },
    ]);
    expect(files.toast?.error).not.toBe(true);
    expect(view.selectedShown()).toEqual([]);
  });

  it('pack takes the folder only, and the selection inside it is let go when the job started', async () => {
    const { files, view } = await world('left', {}, NESTED);
    view.selectAll();

    files.run('pack');

    expect(files.toast).toBeUndefined();
    expect(files.tools.dialog).toMatchObject({ kind: 'pack', path: '/work', names: ['d', 'e/h'] });

    files.toolDone();

    expect(view.selectedShown()).toEqual([]);
  });
});
