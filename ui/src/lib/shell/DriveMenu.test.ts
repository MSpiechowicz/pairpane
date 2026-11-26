import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { Volume } from '$lib/ipc/bindings/Volume';
import { PaneTabs } from '$lib/panes/tabs.svelte';
import { Volumes, VOLUMES_POLL_MS } from '$lib/panes/volumes.svelte';

import { arrive, crumbsTo, fakeBackend, folderStart } from '../../../tests/panes';
import DriveMenu from './DriveMenu.svelte';

const volume = (name: string, path: string, extra: Partial<Volume> = {}): Volume => ({
  name,
  label: null,
  path,
  kind: 'drive',
  free: null,
  ...extra,
});

const places = [
  volume('System', '/', { kind: 'system', free: 40 * 1024 ** 3 }),
  volume('Home', '/home/me', { kind: 'home' }),
  volume('USB', '/media/me/USB', { kind: 'removable', label: 'Photos', free: 2 * 1024 ** 3 }),
];

/** A drive menu for a pane that is in `path` (the crumbs are the whole path from the root). */
async function menu(path = '/home/me/Projects', offered = places) {
  const backend = fakeBackend();
  const tabs = new PaneTabs(backend.ipc, 'left');
  await tabs.view.listing.open('/start');
  arrive(backend.last(), '/start');
  await tabs.view.listing.open(path);
  backend.last().emit(folderStart(path, 0, { parent: '/', crumbs: crumbsTo(path) }));

  const volumes = new Volumes(backend.ipc);
  await volumes.refresh();
  backend.volumes[0].emit({ kind: 'list', volumes: offered.map((place) => ({ ...place, free: null })) });
  for (const place of offered.filter((candidate) => candidate.free !== null)) {
    backend.volumes[0].emit({ kind: 'free', path: place.path, bytes: place.free as number });
  }

  const onClose = vi.fn();
  render(DriveMenu, { tabs, volumes, onClose });
  await tick();
  return { backend, tabs, volumes, onClose };
}

const list = () => screen.getByRole('listbox', { name: 'Drives' });
const options = () => screen.getAllByRole('option');
const highlighted = () => list().getAttribute('aria-activedescendant');

afterEach(() => vi.useRealTimers());

describe('DriveMenu', () => {
  it('shows a volume name and label that reorder text with their characters written out', async () => {
    await menu('/home/me', [volume('Disk\u202Efdp', '/mnt/d', { label: 'Photos\u200Bx' })]);

    expect(options().map((option) => option.textContent?.replace(/\s+/g, ' ').trim())).toEqual(['Disk⟨U+202E⟩fdp Photos⟨U+200B⟩x']);
  });

  it('shows each volume with its dimmed label and its free space when it is known', async () => {
    await menu();

    expect(options().map((option) => option.textContent?.replace(/\s+/g, ' ').trim())).toEqual([
      'System 40 GB free',
      'Home',
      'USB Photos 2.0 GB free',
    ]);
  });

  it('marks the volume the pane is in and starts on it', async () => {
    await menu('/media/me/USB/photos');

    expect(options().map((option) => option.getAttribute('aria-selected'))).toEqual(['false', 'false', 'true']);
    expect(highlighted()).toBe(options()[2].id);
  });

  it('has the keyboard focus, and the arrow keys move the highlight without leaving the ends', async () => {
    await menu('/home/me/Projects');
    expect(list()).toHaveFocus();
    expect(highlighted()).toBe(options()[1].id);

    await fireEvent.keyDown(list(), { key: 'ArrowDown' });
    expect(highlighted()).toBe(options()[2].id);
    await fireEvent.keyDown(list(), { key: 'ArrowDown' });
    expect(highlighted()).toBe(options()[2].id);

    await fireEvent.keyDown(list(), { key: 'ArrowUp' });
    await fireEvent.keyDown(list(), { key: 'ArrowUp' });
    await fireEvent.keyDown(list(), { key: 'ArrowUp' });
    expect(highlighted()).toBe(options()[0].id);
  });

  it('Enter opens the highlighted volume`s root in the current tab, as a step of its history, and closes', async () => {
    const { backend, tabs, onClose } = await menu('/home/me/Projects');
    await fireEvent.keyDown(list(), { key: 'ArrowDown' });

    await fireEvent.keyDown(list(), { key: 'Enter' });

    expect(backend.last().args).toMatchObject({ pane: 'left', path: '/media/me/USB', child: null });
    expect(onClose).toHaveBeenCalledWith(true);

    arrive(backend.last(), '/media/me/USB');
    expect(tabs.canBack).toBe(true);
    tabs.back();
    expect(backend.last().args.path).toBe('/home/me/Projects');
  });

  it('a click chooses a volume as well', async () => {
    const { backend, onClose } = await menu();

    await fireEvent.click(options()[0]);

    expect(backend.last().args).toMatchObject({ path: '/' });
    expect(onClose).toHaveBeenCalledWith(true);
  });

  it('Escape closes and gives the focus back; a click elsewhere closes and leaves it there', async () => {
    const { onClose } = await menu();

    await fireEvent.keyDown(list(), { key: 'Escape' });
    expect(onClose).toHaveBeenLastCalledWith(true);

    await fireEvent.click(document.body);
    expect(onClose).toHaveBeenLastCalledWith(false);
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it('a click on the control that opened the menu is left to that control, and one inside the menu is not outside', async () => {
    const { onClose } = await menu();
    const opener = document.body.appendChild(document.createElement('button'));
    opener.dataset.menuOpener = '';

    await fireEvent.click(opener);
    await fireEvent.click(list());

    expect(onClose).not.toHaveBeenCalled();
  });

  it('reads the volumes again every 2 s while it is open, and stops when it is gone', async () => {
    vi.useFakeTimers();
    const backend = fakeBackend();
    const tabs = new PaneTabs(backend.ipc, 'left');
    const volumes = new Volumes(backend.ipc);

    const { unmount } = render(DriveMenu, { tabs, volumes, onClose: vi.fn() });
    await vi.advanceTimersByTimeAsync(VOLUMES_POLL_MS * 2);
    expect(backend.volumes).toHaveLength(3);

    unmount();
    await vi.advanceTimersByTimeAsync(VOLUMES_POLL_MS * 5);
    expect(backend.volumes).toHaveLength(3);
  });

  it('shows a volume that is mounted while it is open', async () => {
    const { backend } = await menu();
    expect(options()).toHaveLength(3);

    expect(backend.volumes).toHaveLength(2); // opening the menu read the list again
    backend.volumes[1].emit({ kind: 'list', volumes: [...places, volume('New', '/media/me/New')] });
    await tick();

    expect(options()).toHaveLength(4);
  });
});
