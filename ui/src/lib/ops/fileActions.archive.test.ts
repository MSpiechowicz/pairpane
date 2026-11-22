import { describe, expect, it } from 'vitest';

import type { PaneId } from '$lib/ipc/bindings/PaneId';
import { PaneTabs } from '$lib/panes/tabs.svelte';

import { arrive, entry, fakeBackend } from '../../../tests/panes';
import type { FileAction } from './dialogs';
import { FileActions } from './fileActions.svelte';
import { INTO_ARCHIVE_NOTICE, READ_ONLY_NOTICE } from './targets';

const ZIP = { archive: '/work/a.zip', dir: '' };

/** Lists `/work` (a.zip, b.txt, 1.001) in `tabs`, as a real folder or, with `archive`, as the root of `a.zip`. */
async function list(backend: ReturnType<typeof fakeBackend>, tabs: PaneTabs, archive: boolean) {
  const entries = [entry('a.zip'), entry('b.txt'), entry('1.001')];
  if (archive) {
    await tabs.view.listing.openArchive(ZIP);
    backend.last().emit({ kind: 'start', path: '/work', parent: '/', crumbs: [], total: 3, archive: ZIP });
    backend.last().emit({ kind: 'batch', offset: 0, entries });
    backend.last().emit({ kind: 'done', dirs: 0, files: 3, bytes: 0, free: null });
  } else {
    await tabs.view.listing.open('/work');
    arrive(backend.last(), '/work', entries);
  }
}

/** The actions of a window whose left pane is active; `inside` says which panes are inside an archive. */
async function world(inside: Record<PaneId, boolean>) {
  const backend = fakeBackend();
  const panes = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
  await list(backend, panes.left, inside.left);
  await list(backend, panes.right, inside.right);
  panes.left.view.setCursor(2); // b.txt (row 1 is a.zip)

  return { backend, files: new FileActions(backend.ipc, panes, () => 'left') };
}

const CHANGES: FileAction[] = [
  'move',
  'rename',
  'renameInline',
  'newFolder',
  'newFile',
  'trash',
  'deletePermanently',
  'symlink',
  'hardlink',
  'properties',
  'attributes',
  'checksum',
  'verifyChecksum',
  'split',
  'combine',
];

describe('FileActions in an archive', () => {
  it.each(CHANGES)('%s is refused with the read-only notice, and nothing is opened or asked', async (action) => {
    const { backend, files } = await world({ left: true, right: false });
    const calls = backend.invoke.mock.calls.length;

    files.run(action);

    expect(files.toast).toMatchObject({ text: READ_ONLY_NOTICE, error: true });
    expect([files.dialog, files.removal, files.tools.dialog, files.rename.current]).toEqual([undefined, undefined, undefined, undefined]);
    expect(backend.invoke.mock.calls).toHaveLength(calls);
  });

  it('copy is extract: the dialog is for the selection of the archive', async () => {
    const { files } = await world({ left: true, right: false });

    files.run('copy');

    expect(files.toast).toBeUndefined();
    expect(files.dialog?.request).toEqual({ kind: 'extract', archives: [ZIP], names: ['b.txt'], to: '/work' });
  });

  it.each<FileAction>(['copy', 'move', 'symlink', 'hardlink', 'split', 'combine', 'extract'])(
    '%s into a pane that shows an archive is refused',
    async (action) => {
      const { files } = await world({ left: false, right: true });

      files.run(action);

      expect(files.toast).toMatchObject({ text: INTO_ARCHIVE_NOTICE, error: true });
      expect([files.dialog, files.tools.dialog]).toEqual([undefined, undefined]);
    },
  );

  it('an archive pane extracts into an archive pane: refused, not read-only', async () => {
    const { files } = await world({ left: true, right: true });

    files.run('copy');

    expect(files.toast?.text).toBe(INTO_ARCHIVE_NOTICE);
  });

  it('the actions that only read still work on a folder, and delete still asks there', async () => {
    const { files } = await world({ left: false, right: false });

    files.run('trash');
    expect(files.removal?.names).toEqual(['b.txt']);

    files.run('copy');
    expect(files.dialog?.request).toMatchObject({ kind: 'copy', from: '/work', to: '/work' });
    expect(files.toast).toBeUndefined();
  });
});
