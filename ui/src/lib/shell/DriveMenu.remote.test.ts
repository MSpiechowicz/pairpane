import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, expect, it, vi } from 'vitest';

import { PaneTabs } from '$lib/panes/tabs.svelte';
import { Volumes } from '$lib/panes/volumes.svelte';
import { Remote } from '$lib/remote/remote.svelte';

import { arrive, deliverRemote, entry, fakeBackend } from '../../../tests/panes';
import DriveMenu from './DriveMenu.svelte';

/** A drive menu of a pane in `/work`, or on the server 5 with `onServer`; `withRemote` false: an app that cannot connect. */
async function menu(onServer: boolean, withRemote = true) {
  const backend = fakeBackend();
  const panes = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
  await panes.left.view.listing.open('/work');
  arrive(backend.last(), '/work', [entry('a.txt')]);
  if (onServer) {
    await deliverRemote(backend, panes.left.view.listing, '/work', [entry('b.txt')], 5, 'ann@files');
  }

  const volumes = new Volumes(backend.ipc);
  const remote = withRemote ? new Remote(backend.ipc, panes, () => 'left', vi.fn()) : undefined;
  const onClose = vi.fn();
  render(DriveMenu, { tabs: panes.left, volumes, remote, onClose });
  await tick();

  return { backend, panes, remote, onClose };
}

describe('DriveMenu and servers', () => {
  it('offers to connect to an SFTP server, which opens the Connect dialog and closes the menu', async () => {
    const { remote, onClose } = await menu(false);

    await fireEvent.click(screen.getByRole('button', { name: 'Connect to SFTP server…' }));

    expect(remote?.shown).toBe(true);
    expect(onClose).toHaveBeenCalledWith(false);
    expect(screen.queryByRole('button', { name: /Disconnect/ })).toBeNull();
  });

  it('offers to disconnect only for a pane that is on a server, naming it, and takes the pane back', async () => {
    const { backend, panes, onClose } = await menu(true);

    await fireEvent.click(screen.getByRole('button', { name: 'Disconnect from ann@files' }));
    await tick();

    expect(backend.invoke).toHaveBeenCalledWith('remote_disconnect', { site: 5 });
    expect(onClose).toHaveBeenCalledWith(true);
    expect(backend.last().args.path).toBe('/work');
    expect(panes.left.tabs[0].remote).toBeUndefined();
  });

  it('marks no drive as the pane’s when it is on a server whose folders are named like drives', async () => {
    await menu(true);

    expect(screen.queryAllByRole('option', { selected: true })).toEqual([]);
  });

  it('offers neither without a way to connect', async () => {
    await menu(false, false);

    expect(screen.queryByRole('button', { name: /SFTP|Disconnect/ })).toBeNull();
  });
});
