import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, expect, it, vi } from 'vitest';

import type { Volume } from '$lib/ipc/bindings/Volume';
import { PaneTabs } from '$lib/panes/tabs.svelte';

import { arrive, deliverRemote, entry, fakeBackend } from '../../../tests/panes';
import PaneHeader from './PaneHeader.svelte';

/** A volume at the path `/home`, which is also the server's folder. */
const home: Volume = { name: 'Home', label: null, path: '/home', kind: 'home', free: null };

async function header() {
  const backend = fakeBackend();
  const tabs = new PaneTabs(backend.ipc, 'left');
  await tabs.view.listing.open('/work');
  arrive(backend.last(), '/work', [entry('a.txt')]);
  await deliverRemote(backend, tabs.view.listing, '/home/ann/docs', [], 5, 'ann@files');
  render(PaneHeader, { tabs, volumes: [home], driveMenuOpen: false, onDriveMenu: vi.fn() });

  return { backend, tabs, path: screen.getByRole('navigation', { name: 'Path' }) };
}

describe('PaneHeader on a server', () => {
  it('starts the path at the server’s label, whatever drive of this computer has the same path', async () => {
    const { path } = await header();

    expect(within(path).getAllByRole('button').map((button) => button.textContent)).toEqual(['ann@files', 'home', 'ann', 'docs']);
  });

  it('opens the folder of a step on the same server, the label’s step being its root', async () => {
    const { backend, path } = await header();

    await fireEvent.click(within(path).getByRole('button', { name: 'ann' }));
    expect(backend.last().args).toMatchObject({ site: 5, dir: '/home/ann', child: null });

    await fireEvent.click(within(path).getByRole('button', { name: 'ann@files' }));
    expect(backend.last().args).toMatchObject({ site: 5, dir: '/', child: null });
    expect(backend.invoke).not.toHaveBeenCalledWith('pane_open', expect.objectContaining({ path: '/home/ann' }));
  });

  it('names another tab that is on a server by its folder', async () => {
    const { tabs } = await header();

    tabs.add();
    await tick();
    const others = within(screen.getByRole('group', { name: 'Tabs' })).getAllByRole('button', { name: 'docs' });

    expect(others.filter((button) => !screen.getByRole('navigation', { name: 'Path' }).contains(button))).toHaveLength(1); // the first tab; the second one is the path
  });
});
