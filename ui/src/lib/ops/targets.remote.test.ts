import { describe, expect, it } from 'vitest';

import { PaneView } from '$lib/panes/view.svelte';

import { deliver, deliverRemote, entry, fakeBackend } from '../../../tests/panes';
import type { FileAction } from './dialogs';
import { READ_ONLY_NOTICE, refusal, REMOTE_NOTICE } from './targets';

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

describe('refusal with a pane on a server', () => {
  it.each(ACTIONS)('refuses %s from the server with the notice about servers, not the archive one', async (action) => {
    const [remote, disk] = [await server(), await local()];

    expect(refusal(action, remote, disk)).toBe(REMOTE_NOTICE);
    expect(refusal(action, remote, remote)).toBe(REMOTE_NOTICE);
  });

  it('does not treat copying out of a server as extracting (a read-only listing would be)', async () => {
    const [remote, disk] = [await server(), await local()];

    expect(disk.listing.readonly).toBe(false);
    expect(remote.listing.readonly).toBe(true);
    expect(refusal('copy', remote, disk)).not.toBe(READ_ONLY_NOTICE);
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
