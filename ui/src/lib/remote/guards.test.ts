import { describe, expect, it, vi } from 'vitest';

import { Comparison } from '$lib/compare/controller.svelte';
import { pickFiles } from '$lib/diff/pick';
import { FileActions } from '$lib/ops/fileActions.svelte';
import { REMOTE_NOTICE } from '$lib/ops/targets';
import { extraTargetOf, gitPlaceOf } from '$lib/panes/extras';
import { FlatView } from '$lib/panes/flat.svelte';
import { PaneTabs } from '$lib/panes/tabs.svelte';
import { targetOf } from '$lib/panes/thumbs';
import { placeOf } from '$lib/shell/command-line.svelte';
import { Viewer } from '$lib/viewer/viewer.svelte';

import { arrive, deliverRemote, entry, fakeBackend } from '../../../tests/panes';

/** The left pane is on a server (`/srv`: docs, a.txt), the right one in `/work` (b.txt). */
async function world() {
  const backend = fakeBackend();
  const panes = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
  await deliverRemote(backend, panes.left.view.listing, '/srv', [entry('docs', 'dir'), entry('a.txt')]);
  await panes.right.view.listing.open('/work');
  arrive(backend.last(), '/work', [entry('b.txt')]);
  panes.left.view.setCursor(2);
  panes.right.view.setCursor(1);
  const calls = backend.invoke.mock.calls.length;

  return { backend, panes, left: panes.left.view, right: panes.right.view, untouched: () => backend.invoke.mock.calls.length === calls };
}

describe('what a pane on a server refuses', () => {
  it('the file keys: a card, and no dialog, request or inline field', async () => {
    const { panes, backend, untouched } = await world();
    const files = new FileActions(backend.ipc, panes, () => 'left');

    files.run('copy');
    files.run('trash');
    files.run('renameInline');

    expect(files.toast).toMatchObject({ text: REMOTE_NOTICE, error: true });
    expect([files.dialog, files.removal, files.tools.dialog, files.rename.current]).toEqual([undefined, undefined, undefined, undefined]);
    expect(untouched()).toBe(true);
  });

  it('F4, with the same card', async () => {
    const { panes, backend } = await world();
    const files = new FileActions(backend.ipc, panes, () => 'left');

    expect(files.refuseEdit()).toBe(true);
    expect(files.toast?.text).toBe(REMOTE_NOTICE);
  });

  it('copying to the other pane when that one is the server', async () => {
    const { panes, backend } = await world();
    const files = new FileActions(backend.ipc, panes, () => 'right');

    files.run('copy');

    expect(files.toast).toMatchObject({ text: REMOTE_NOTICE, error: true });
    expect(files.dialog).toBeUndefined();
  });

  it('the quick view and F3 (the Viewer itself): there is nothing to show', async () => {
    const { panes, backend, untouched } = await world();
    const viewer = new Viewer(backend.ipc, panes, () => 'left');

    expect(viewer.under).toBeUndefined();
    viewer.open();

    expect(viewer.overlay).toBeUndefined();
    expect(untouched()).toBe(true);
  });

  it('Shift+F2 for a file on a server, in either pane', async () => {
    const { left, right } = await world();

    expect(pickFiles({ active: left, other: right, side: 'left', quick: false })).toBe(REMOTE_NOTICE);
    expect(pickFiles({ active: right, other: left, side: 'right', quick: false })).toBe(REMOTE_NOTICE);
  });

  it('comparing and synchronizing folders', async () => {
    const { panes, backend, untouched } = await world();
    const say = vi.fn();
    const comparison = new Comparison(backend.ipc, panes, say, () => false);

    comparison.quick();
    comparison.synchronize();

    expect(say.mock.calls).toEqual([
      [REMOTE_NOTICE, true],
      [REMOTE_NOTICE, true],
    ]);
    expect(comparison.modal).toBe(false);
    expect(untouched()).toBe(true);
  });

  it('a flat view', async () => {
    const { panes, backend, untouched } = await world();
    const say = vi.fn();

    new FlatView(backend.ipc, panes, say, () => false).left.toggle();

    expect(say).toHaveBeenCalledWith(REMOTE_NOTICE, true);
    expect(untouched()).toBe(true);
  });

  it('a command line: a refusal in place of a folder', async () => {
    const { panes } = await world();

    expect(placeOf(panes.left.view.listing)).toEqual({ refusal: REMOTE_NOTICE });
    expect(placeOf(panes.right.view.listing)).toEqual({ folder: '/work' });
  });

  it('thumbnails, extra columns and git status: no file on disk is named for any entry', async () => {
    const { left } = await world();
    const file = left.entryAt(2);

    expect(targetOf(left.listing, file, 128)).toBeUndefined();
    expect(extraTargetOf(left.listing, file)).toBeUndefined();
    expect(gitPlaceOf(left.listing, file)).toBeUndefined();
  });
});
