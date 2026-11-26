import { describe, expect, it } from 'vitest';

import { PaneView } from '$lib/panes/view.svelte';

import { deliver, deliverRemote, entry, fakeBackend } from '../../../tests/panes';
import { reloadRemote } from './transfer';

/** A pane that shows `dir` on `site`, and the backend that has been asked to list it. */
async function shows(dir: string, site = 1) {
  const backend = fakeBackend();
  const view = new PaneView(backend.ipc, 'left');
  await deliverRemote(backend, view.listing, dir, [entry('a.txt')], site);
  return { backend, view };
}

/** How many times the pane has been asked to list a folder on a server. */
const opened = (backend: ReturnType<typeof fakeBackend>) => backend.invoke.mock.calls.filter(([name]) => name === 'remote_open').length;

describe('reloadRemote', () => {
  it.each(['/srv', '/srv/deeper/still'])('lists the pane again when it shows the job’s folder on that server or one below it (%s)', async (dir) => {
    const { backend, view } = await shows(dir);

    reloadRemote([{ view }], { site: 1, dir: '/srv' });

    expect(opened(backend)).toBe(2);
  });

  it('lists every folder again below the root', async () => {
    const { backend, view } = await shows('/anywhere');

    reloadRemote([{ view }], { site: 1, dir: '/' });

    expect(opened(backend)).toBe(2);
  });

  it.each([
    ['a folder that only starts with the same letters', '/srvx', 1],
    ['the folder above', '/', 1],
    ['a sibling', '/other', 1],
    ['the same folder on another server', '/srv', 2],
  ])('leaves a pane that shows %s alone', async (_what, dir, site) => {
    const { backend, view } = await shows(dir, site);

    reloadRemote([{ view }], { site: 1, dir: '/srv' });

    expect(opened(backend)).toBe(1);
  });

  it('leaves a pane on a folder of this computer alone, and does nothing for a job with no server side', async () => {
    const backend = fakeBackend();
    const disk = new PaneView(backend.ipc, 'right');
    await deliver(backend, () => disk.listing.open('/srv'), '/srv', [entry('a.txt')]);
    const { backend: other, view } = await shows('/srv');

    reloadRemote([{ view: disk }], { site: 1, dir: '/srv' });
    reloadRemote([{ view }], undefined);

    expect(backend.invoke.mock.calls.filter(([name]) => name === 'pane_open')).toHaveLength(1);
    expect(opened(other)).toBe(1);
  });
});
