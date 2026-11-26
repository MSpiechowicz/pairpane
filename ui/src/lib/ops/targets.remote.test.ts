import { describe, expect, it } from 'vitest';

import { PaneView } from '$lib/panes/view.svelte';

import { archiveStart, deliver, deliverRemote, entry, fakeBackend } from '../../../tests/panes';
import type { FileAction } from './dialogs';
import { refusal, REMOTE_NOTICE, requestFor } from './targets';

const ACTIONS: FileAction[] = [
  'copy',
  'move',
  'rename',
  'renameInline',
  'multiRename',
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
  'extract',
  'pack',
];

/** A pane that has listed `path` on a folder on disk. */
async function local(path = '/work') {
  const backend = fakeBackend();
  const view = new PaneView(backend.ipc, 'right');
  await deliver(backend, () => view.listing.open(path), path, [entry('a.txt')]);
  return view;
}

/** A pane that has listed `/srv` on a server. */
async function server() {
  const backend = fakeBackend();
  const view = new PaneView(backend.ipc, 'left');
  await deliverRemote(backend, view.listing, '/srv', [entry('a.txt')]);
  return view;
}

/** A pane that shows the folder `dir` of the archive `/work/a.zip`. */
async function inArchive() {
  const backend = fakeBackend();
  const view = new PaneView(backend.ipc, 'right');
  const spot = { archive: '/work/a.zip', dir: '' };
  await view.listing.openArchive(spot);
  backend.last().emit(archiveStart('/work', 1, spot));
  backend.last().emit({ kind: 'batch', offset: 0, entries: [entry('a.txt')] });
  backend.last().emit({ kind: 'done', dirs: 0, files: 1, bytes: 0, free: null });
  return view;
}

/** What a server pane and a folder pane have apart from copying and moving, which go to the folder on this computer. */
const OTHERS = ACTIONS.filter((action) => action !== 'copy' && action !== 'move');

describe('refusal with a pane on a server', () => {
  it.each(OTHERS)('refuses %s from the server with the notice about servers, not the archive one', async (action) => {
    const [remote, disk] = [await server(), await local()];

    expect(refusal(action, remote, disk)).toBe(REMOTE_NOTICE);
    expect(refusal(action, remote, remote)).toBe(REMOTE_NOTICE);
  });

  it.each(['copy', 'move'] as const)('lets %s from the server into a folder on this computer through', async (action) => {
    const [remote, disk] = [await server(), await local()];

    expect(refusal(action, remote, disk)).toBeUndefined();
  });

  it.each(['copy', 'move'] as const)('refuses %s from the server into another server pane, an archive or a folder not listed yet', async (action) => {
    const remote = await server();
    const unlisted = new PaneView(fakeBackend().ipc, 'right');

    expect(refusal(action, remote, remote)).toBe(REMOTE_NOTICE);
    expect(refusal(action, remote, await inArchive())).toBe(REMOTE_NOTICE);
    expect(refusal(action, remote, unlisted)).toBe(REMOTE_NOTICE);
  });

  it('says that copying and moving to this computer work, in the notice', () => {
    expect(REMOTE_NOTICE).toBe('Files on a server can only be browsed and copied or moved to this computer for now.');
  });

  it('does not treat copying out of a server as extracting (a read-only listing would be)', async () => {
    const [remote, disk] = [await server(), await local()];

    expect(disk.listing.readonly).toBe(false);
    expect(remote.listing.readonly).toBe(true);
    remote.setCursor(1);
    expect(refusal('copy', remote, disk)).toBeUndefined();
    expect(requestFor('copy', remote, disk)?.kind).toBe('remote');
    expect(refusal('extract', remote, disk)).toBe(REMOTE_NOTICE);
  });

  it.each(['copy', 'move', 'symlink', 'hardlink', 'split', 'combine', 'extract'] as const)(
    'refuses %s into a pane on a server',
    async (action) => {
      const [remote, disk] = [await server(), await local()];

      expect(refusal(action, disk, remote)).toBe(REMOTE_NOTICE);
    },
  );

  it('lets the actions that only touch the pane’s own folder through when the other pane is on a server', async () => {
    const [remote, disk] = [await server(), await local()];

    expect(refusal('trash', disk, remote)).toBeUndefined();
    expect(refusal('newFolder', disk, remote)).toBeUndefined();
    expect(refusal('copy', disk, disk)).toBeUndefined();
  });
});

describe('the request for copying or moving out of a server', () => {
  it.each([
    ['copy', false],
    ['move', true],
  ] as const)('%s names the folder on the server, the folder here and the chosen items', async (kind, move) => {
    const [remote, disk] = [await server(), await local('/work')];
    remote.setCursor(1);

    expect(requestFor(kind, remote, disk)).toEqual({
      kind: 'remote',
      move,
      from: { kind: 'remote', site: 1, dir: '/srv' },
      to: { kind: 'folder', path: '/work' },
      names: ['a.txt'],
      shown: '/work',
    });
  });

  it('is not made for the other keys, a pane that is no plain folder, or when nothing is chosen', async () => {
    const [remote, disk] = [await server(), await local()];
    remote.setCursor(1);

    expect(requestFor('extract', remote, disk)).toBeUndefined();
    expect(requestFor('mkdir', remote, disk)).toBeUndefined();
    expect(requestFor('copy', remote, await inArchive())).toBeUndefined();

    remote.setCursor(0); // the `..` row
    expect(requestFor('copy', remote, disk)).toBeUndefined();
  });

  it('is not made for copying into a server: that stays a copy of local items, which refusal has refused', async () => {
    const [remote, disk] = [await server(), await local()];
    disk.setCursor(1);

    expect(requestFor('copy', disk, remote)?.kind).toBe('copy');
    expect(refusal('copy', disk, remote)).toBe(REMOTE_NOTICE);
  });
});
