import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, expect, it, vi } from 'vitest';

import { Hotlist } from '$lib/panes/hotlist.svelte';
import { PaneTabs } from '$lib/panes/tabs.svelte';

import { arrive, fakeBackend } from '../../../tests/panes';
import HotlistMenu from './HotlistMenu.svelte';

/** A hotlist menu for a pane that is in `/home/me/Projects`. */
async function menu(favourites: string[] = ['/home/me/Music', '/media/USB', '/home/me/Projects/pairpane']) {
  const backend = fakeBackend();
  const tabs = new PaneTabs(backend.ipc, 'left');
  await tabs.view.listing.open('/home/me/Projects');
  arrive(backend.last(), '/home/me/Projects');

  const hotlist = new Hotlist();
  hotlist.restore(favourites);
  const onClose = vi.fn();
  render(HotlistMenu, { tabs, hotlist, onClose });
  await tick();
  return { backend, tabs, hotlist, onClose };
}

const field = () => screen.getByRole('combobox', { name: 'Filter favourite folders' });
const options = () => screen.queryAllByRole('option').map((option) => option.textContent?.trim());
const toggle = () => screen.getByRole('button', { name: /current folder$/ });

describe('HotlistMenu', () => {
  it('lists the favourites in the order they were added; the filter field has the focus', async () => {
    await menu();

    expect(options()).toEqual(['/home/me/Music', '/media/USB', '/home/me/Projects/pairpane']);
    expect(field()).toHaveFocus();
  });

  it('typing filters by a case-insensitive part of the path and highlights the first match', async () => {
    await menu();

    await fireEvent.input(field(), { target: { value: 'PROJ' } });

    expect(options()).toEqual(['/home/me/Projects/pairpane']);
    expect(field().getAttribute('aria-activedescendant')).toBe(screen.getByRole('option').id);
  });

  it('says so when nothing matches, or when there are no favourites at all', async () => {
    await menu([]);
    expect(screen.getByText('No favourite folders yet.')).toBeInTheDocument();

    await fireEvent.input(field(), { target: { value: 'x' } });
    expect(screen.getByText('No favourite folder matches.')).toBeInTheDocument();
  });

  it('Enter opens the highlighted favourite in the pane and closes', async () => {
    const { backend, onClose } = await menu();
    await fireEvent.keyDown(field(), { key: 'ArrowDown' });

    await fireEvent.keyDown(field(), { key: 'Enter' });

    expect(backend.last().args).toMatchObject({ pane: 'left', path: '/media/USB', child: null });
    expect(onClose).toHaveBeenCalledWith(true);
  });

  it('Enter opens the first match of a filter', async () => {
    const { backend } = await menu();
    await fireEvent.input(field(), { target: { value: 'usb' } });

    await fireEvent.keyDown(field(), { key: 'Enter' });

    expect(backend.last().args).toMatchObject({ path: '/media/USB' });
  });

  it('a click opens a favourite', async () => {
    const { backend } = await menu();

    await fireEvent.click(screen.getByRole('option', { name: '/home/me/Music' }));

    expect(backend.last().args).toMatchObject({ path: '/home/me/Music' });
  });

  it('Escape closes and gives the focus back', async () => {
    const { onClose } = await menu();

    await fireEvent.keyDown(field(), { key: 'Escape' });

    expect(onClose).toHaveBeenCalledWith(true);
  });

  it('adds the current folder, then offers to remove it; nothing asks for confirmation', async () => {
    const { hotlist } = await menu([]);

    expect(toggle()).toHaveTextContent('Add current folder');
    await fireEvent.click(toggle());
    expect(hotlist.paths).toEqual(['/home/me/Projects']);
    expect(options()).toEqual(['/home/me/Projects']);

    expect(toggle()).toHaveTextContent('Remove current folder');
    await fireEvent.click(toggle());
    expect(hotlist.paths).toEqual([]);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('after the footer button the filter has the focus again, so the keys keep working', async () => {
    const { hotlist } = await menu(['/media/USB']);
    await fireEvent.click(toggle());
    toggle().focus(); // a click focuses the button it hits

    await fireEvent.click(toggle());

    expect(hotlist.paths).toEqual(['/media/USB']);
    expect(field()).toHaveFocus();

    await fireEvent.keyDown(field(), { key: 'ArrowDown' });
    expect(field().getAttribute('aria-activedescendant')).toBe(screen.getAllByRole('option')[0].id);
  });

  it('after the footer button the filter has the focus again, so the arrow keys keep working', async () => {
    const { hotlist } = await menu(['/media/USB']);
    toggle().focus(); // a click focuses the button it hits

    await fireEvent.click(toggle());

    expect(hotlist.paths).toEqual(['/media/USB', '/home/me/Projects']);
    expect(field()).toHaveFocus();

    await fireEvent.keyDown(field(), { key: 'ArrowDown' });
    expect(field().getAttribute('aria-activedescendant')).toBe(screen.getAllByRole('option')[1].id);
  });

  it('removes a favourite with its × or with Shift+Delete, and stays open', async () => {
    const { hotlist, onClose } = await menu();

    await fireEvent.click(screen.getAllByRole('button', { name: 'Remove from favourites' })[0]);
    expect(hotlist.paths).toEqual(['/media/USB', '/home/me/Projects/pairpane']);

    await fireEvent.keyDown(field(), { key: 'Delete', shiftKey: true });
    expect(hotlist.paths).toEqual(['/home/me/Projects/pairpane']);
    expect(onClose).not.toHaveBeenCalled();
  });

  it('Delete alone edits the text and removes nothing', async () => {
    const { hotlist } = await menu();

    expect(await fireEvent.keyDown(field(), { key: 'Delete' })).toBe(true);

    expect(hotlist.paths).toHaveLength(3);
  });

  it('cannot add a folder inside an archive', async () => {
    const { backend, tabs } = await menu();
    await tabs.view.listing.openArchive({ archive: '/home/me/Projects/a.zip', dir: '' });
    backend.last().emit({
      kind: 'start',
      path: '/home/me/Projects',
      parent: '/home/me',
      crumbs: [],
      total: 0,
      archive: { archive: '/home/me/Projects/a.zip', dir: '' },
    });
    await tick();

    expect(toggle()).toBeDisabled();
    expect(toggle()).toHaveAttribute('title', 'A folder inside an archive can’t be a favourite.');
  });
});
