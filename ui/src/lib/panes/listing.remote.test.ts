import { describe, expect, it } from 'vitest';

import { arrive, deliverRemote, entry, fakeBackend, folderStart, remoteStart } from '../../../tests/panes';
import { Listing } from './listing.svelte';
import { PaneView } from './view.svelte';

/** A pane that has listed `/home/me` on the server 7, called `me@host`. */
async function onServer(entries = [entry('docs', 'dir'), entry('a.txt')]) {
  const backend = fakeBackend();
  const view = new PaneView(backend.ipc, 'left');
  await deliverRemote(backend, view.listing, '/home/me', entries, 7);
  return { backend, view };
}

describe('a listing on a server', () => {
  it('asks the server for the folder and has neither a path nor a folder on disk, and cannot be changed', async () => {
    const { backend, view } = await onServer();

    expect(backend.opens[0].args).toMatchObject({ pane: 'left', site: 7, dir: '/home/me', child: null, showHidden: false });
    expect(view.listing.remote).toEqual({ kind: 'remote', site: 7, label: 'me@host', dir: '/home/me' });
    expect(view.listing.path).toBeUndefined();
    expect(view.listing.folder).toBeUndefined();
    expect(view.listing.readonly).toBe(true);
    expect(view.listing.parent).toBe('/home');
    expect(view.listing.crumbs.map((crumb) => crumb.name)).toEqual(['me@host', 'home', 'me']);
  });

  it('enters a folder by asking the server for it as a child of the folder shown', async () => {
    const { backend, view } = await onServer();

    await view.open(1); // docs

    expect(backend.last().args).toMatchObject({ site: 7, dir: '/home/me', child: 'docs' });
    expect(backend.invoke).not.toHaveBeenCalledWith('pane_open', expect.anything());
  });

  it('goes up, with the cursor on the folder it came from, and not above the root', async () => {
    const { backend, view } = await onServer([entry('docs', 'dir')]);

    await view.goUp();
    expect(backend.last().args).toMatchObject({ site: 7, dir: '/home', child: null });
    const channel = backend.last();
    channel.emit(remoteStart(7, 'me@host', '/home', 2));
    channel.emit({ kind: 'batch', offset: 0, entries: [entry('me', 'dir'), entry('you', 'dir')] });
    expect(view.entryAt(view.cursor)?.name).toBe('me');

    await view.goUp();
    backend.last().emit(remoteStart(7, 'me@host', '/', 0));
    expect(view.listing.parent).toBeNull();
    expect(view.up).toBe(false);

    const opens = backend.opens.length;
    await view.goUp();
    expect(backend.opens).toHaveLength(opens); // nothing above the root
  });

  it('opens the folder of a crumb on the same server, the server’s own label at its root', async () => {
    const { backend, view } = await onServer();
    const [root, home] = view.listing.crumbs;

    await view.listing.openCrumb(home);
    expect(backend.last().args).toMatchObject({ site: 7, dir: '/home', child: null });

    await view.listing.openCrumb(root);
    expect(backend.last().args).toMatchObject({ site: 7, dir: '/', child: null });
  });

  it('lists the same folder again as a reload (hidden files toggled), keeping the rows meanwhile', async () => {
    const { backend, view } = await onServer();

    await view.listing.setShowHidden(true);
    const channel = backend.last();
    expect(channel.args).toMatchObject({ site: 7, dir: '/home/me', child: null, showHidden: true });
    channel.emit(remoteStart(7, 'me@host', '/home/me', 2));

    expect(view.listing.reloads).toBe(1);
    expect(view.listing.entries[0]?.name).toBe('docs');
  });

  it('is another place than a folder on disk of the same path, and going there is no reload', async () => {
    const { backend, view } = await onServer();
    const server = view.listing.location;

    await view.listing.open('/home/me');
    backend.last().emit(folderStart('/home/me', 0, { parent: '/home', crumbs: [] }));

    expect(view.listing.reloads).toBe(0);
    expect(view.listing.remote).toBeUndefined();
    expect(view.listing.path).toBe('/home/me');
    expect(view.listing.location).not.toBe(server);
  });

  it('reads the open of a folder on disk as another place than the server’s with the same path', async () => {
    const backend = fakeBackend();
    const listing = new Listing(backend.ipc, 'left');
    await listing.open('/home/me');
    arrive(backend.last(), '/home/me');

    await listing.openRemote({ kind: 'remote', site: 3, label: 'a@b', dir: '/home/me' });
    backend.last().emit(remoteStart(3, 'a@b', '/home/me', 0));

    expect(listing.reloads).toBe(0);
    expect(listing.remote?.site).toBe(3);
  });

  it('keeps the folder shown, and says why, when the server refuses another one', async () => {
    const backend = fakeBackend({ remote_open: ((args: { child: string | null }) => (args.child === null ? null : Promise.reject({ code: 'forbidden', message: 'Permission denied' }))) as never });
    const view = new PaneView(backend.ipc, 'left');
    await deliverRemote(backend, view.listing, '/home/me', [entry('secret', 'dir')], 7);
    const opens = backend.opens.length;

    await view.open(1);

    expect(view.listing.notice).toBe('Permission denied');
    expect(view.listing.remote?.dir).toBe('/home/me');
    expect(backend.opens).toHaveLength(opens + 1); // no quiet relisting to bring a watch back: a server has none
  });

  it('measures no folder size on a server', async () => {
    const { backend, view } = await onServer();

    view.space(); // the cursor is on `..`; move to the folder
    view.setCursor(1);
    view.space();

    expect(backend.sizes).toHaveLength(0);
  });
});
