import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, expect, it, vi } from 'vitest';

import type { Volume } from '$lib/ipc/bindings/Volume';
import { PaneTabs } from '$lib/panes/tabs.svelte';

import { fakeBackend } from '../../../tests/panes';
import PaneHeader from './PaneHeader.svelte';

const home: Volume = { name: 'Home', label: null, path: '/home/me', kind: 'home', free: null };

/** A header for a pane that listed `path` (the crumbs are the whole path from the root). */
function header(path: string, volumes: Volume[] = [home]) {
  const backend = fakeBackend();
  const tabs = new PaneTabs(backend.ipc, 'left');
  void tabs.view.listing.open(path);

  const parts = path.split('/').filter((part) => part !== '');
  backend.last().emit({
    kind: 'start',
    path,
    parent: '/',
    crumbs: [
      { name: '/', path: '/' },
      ...parts.map((name, index) => ({ name, path: `/${parts.slice(0, index + 1).join('/')}` })),
    ],
    total: 0,
  });

  const onDriveMenu = vi.fn();
  render(PaneHeader, { tabs, volumes, driveMenuOpen: false, onDriveMenu });
  return { backend, tabs, onDriveMenu };
}

const path = () => screen.getByRole('navigation', { name: 'Path' });
const group = () => screen.getByRole('group', { name: 'Tabs' });

/** The buttons of the tabs that are not the active one (the path's own buttons are not among them). */
const otherTabs = (name: string) =>
  within(group())
    .getAllByRole('button', { name })
    .filter((button) => !path().contains(button));

describe('PaneHeader', () => {
  it('the active tab is the path, starting at the volume`s name; a click opens that folder', async () => {
    const { backend } = header('/home/me/Projects/pairpane');

    expect(
      within(path())
        .getAllByRole('button')
        .map((button) => button.textContent),
    ).toEqual(['Home', 'Projects', 'pairpane']);

    await fireEvent.click(within(path()).getByRole('button', { name: 'Projects' }));
    expect(backend.last().args).toMatchObject({ path: '/home/me/Projects', child: null });
  });

  it('search results end the path with a mark; the folder searched stays a button that opens it', async () => {
    const { backend, tabs } = header('/home/me/Projects');

    void tabs.view.listing.openResults(4);
    backend.last().emit({
      kind: 'start',
      path: '/home/me/Projects',
      parent: '/home/me',
      crumbs: [
        { name: '/', path: '/' },
        { name: 'Projects', path: '/home/me/Projects' },
      ],
      total: 0,
      results: 4,
    });
    await tick();

    expect(path()).toHaveTextContent('Search results');
    expect(within(path()).queryByRole('button', { name: 'Search results' })).not.toBeInTheDocument();

    await fireEvent.click(within(path()).getByRole('button', { name: 'Projects' }));
    expect(backend.last().args).toMatchObject({ path: '/home/me/Projects', child: null });
  });

  it('a path outside every volume starts at the root, named System', () => {
    header('/etc', []);

    expect(
      within(path())
        .getAllByRole('button')
        .map((button) => button.textContent),
    ).toEqual(['System', 'etc']);
  });

  it('the drive icon asks for the drive menu', async () => {
    const { onDriveMenu } = header('/home/me');

    await fireEvent.click(screen.getByRole('button', { name: 'Drives' }));

    expect(onDriveMenu).toHaveBeenCalledTimes(1);
  });

  it('+ adds a tab; the other tabs show their folder`s name', async () => {
    const { tabs } = header('/home/me/Projects');
    expect(within(group()).queryByRole('button', { name: /^Close/ })).not.toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: 'New tab' }));
    expect(tabs.tabs).toHaveLength(2);
    expect(tabs.active).toBe(1);

    expect(otherTabs('Projects')).toHaveLength(1); // the first tab, now inactive
    expect(within(path()).getAllByRole('button')).toHaveLength(2); // Home, Projects: the new tab is the path
  });

  it('clicking another tab shows it; × and the middle button close it', async () => {
    const { backend, tabs } = header('/home/me/Projects');
    tabs.add();
    tabs.add();
    await tick();
    expect(tabs.active).toBe(2);

    await fireEvent.click(otherTabs('Projects')[0]);
    expect(tabs.active).toBe(0);
    expect(backend.last().args).toMatchObject({ path: '/home/me/Projects' });

    await fireEvent.click(within(group()).getAllByRole('button', { name: 'Close Projects' })[0]);
    expect(tabs.tabs).toHaveLength(2);

    await fireEvent(
      otherTabs('Projects')[0],
      new MouseEvent('auxclick', { button: 1, bubbles: true, cancelable: true }),
    );
    expect(tabs.tabs).toHaveLength(1);
  });

  it('other mouse buttons do not close a tab', async () => {
    const { tabs } = header('/home/me');
    tabs.add();
    await tick();

    await fireEvent(otherTabs('Home')[0], new MouseEvent('auxclick', { button: 2, bubbles: true, cancelable: true }));

    expect(tabs.tabs).toHaveLength(2);
  });

  it('is a plain group of buttons: no tab roles, and no button inside another', async () => {
    const { tabs } = header('/home/me');
    tabs.add();
    await tick();

    expect(screen.queryByRole('tab')).not.toBeInTheDocument();
    expect(screen.queryByRole('tablist')).not.toBeInTheDocument();
    for (const button of screen.getAllByRole('button')) {
      expect(button.querySelector('button')).toBeNull();
    }
  });
});
