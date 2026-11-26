import { describe, expect, it } from 'vitest';

import { arrive, deliverRemote, entry } from '../../../tests/panes';
import { go, inA } from '../../../tests/tabs';
import type { RemotePlace } from './remote';

const PLACE: RemotePlace = { kind: 'remote', site: 4, label: 'me@host', dir: '/srv' };

/** Tabs that have listed `/a` and then the server's `/srv`, in one tab. */
async function onServer() {
  const world = await inA();
  await deliverRemote(world.backend, world.tabs.view.listing, PLACE.dir, [entry('www', 'dir')], PLACE.site, PLACE.label);
  return world;
}

describe('PaneTabs on a server', () => {
  it('saves the last folder on disk for a tab that is on a server, and records no history of the server', async () => {
    const { tabs } = await onServer();

    expect(tabs.state[0].path).toBe('/a');
    expect(tabs.canBack).toBe(false);
  });

  it('records the folders on disk it left for and came back to, not the server', async () => {
    const { backend, tabs } = await onServer();

    await go(backend, tabs, '/b');

    expect(tabs.canBack).toBe(true);
    tabs.back();
    expect(backend.last().args.path).toBe('/a'); // not the server's folder
  });

  it('keeps the place on the server in memory: another tab, and back to this one, lists it again', async () => {
    const { backend, tabs } = await onServer();

    tabs.add();
    await go(backend, tabs, '/b');
    tabs.select(0);

    expect(backend.last().args).toMatchObject({ site: 4, dir: '/srv', child: null });
    expect(tabs.tabs[0].remote).toEqual(PLACE);
    expect(tabs.tabs[1].path).toBe('/b');
    expect(tabs.state.map((tab) => tab.path)).toEqual(['/a', '/b']);
  });

  it('shows a new tab on the server too, as it shows the folder', async () => {
    const { tabs } = await onServer();

    tabs.add();

    expect(tabs.tabs[1].remote).toEqual(PLACE);
    expect(tabs.tabs[1].path).toBe('/a');
  });

  it('lists the last folder on disk when the server cannot be listed again', async () => {
    const { backend, tabs } = await onServer();
    tabs.add();
    await go(backend, tabs, '/b');
    backend.invoke.mockImplementationOnce(() => Promise.reject({ code: 'io', message: 'Connection closed' }));

    tabs.select(0);
    await Promise.resolve();
    await Promise.resolve();

    expect(backend.last().args.path).toBe('/a');
    expect(tabs.tabs[0].remote).toBeUndefined();
    arrive(backend.last(), '/a');
    expect(tabs.state[0].path).toBe('/a');
  });

  it('goes back to the last folder on disk when its connection is dropped, in the tab shown and in the others', async () => {
    const { backend, tabs } = await onServer();
    tabs.add(); // the second tab is on the server too
    expect(tabs.tabs[1].remote).toEqual(PLACE);

    tabs.dropSite(4);

    expect(backend.last().args.path).toBe('/a');
    expect(tabs.tabs.map((tab) => tab.remote)).toEqual([undefined, undefined]);
    arrive(backend.last(), '/a');
    expect(tabs.view.listing.remote).toBeUndefined();
  });

  it('leaves the tabs on another server alone when a connection is dropped', async () => {
    const { backend, tabs } = await onServer();
    const opens = backend.opens.length;

    tabs.dropSite(9);

    expect(backend.opens).toHaveLength(opens);
    expect(tabs.view.listing.remote?.site).toBe(4);
  });
});
