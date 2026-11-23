import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import type { SavedState } from '$lib/ipc/bindings/SavedState';
import { DEFAULT_PREFERENCES } from '$lib/shell/preferences.svelte';

import App from './App.svelte';
import { type Answers, appWorld, pane } from '../tests/app';
import { entry } from '../tests/panes';

const HEX = 'fedcba9876543210'.repeat(4);

const left = () => within(pane('Left pane'));
const settings = () => screen.getByRole('dialog', { name: 'Settings' });

/** The app with a picture, a text file and a folder in the left pane; the backend knows the picture is 640×480. */
async function shown(answers: Answers = {}) {
  const backend = appWorld(invoke, {
    columns_dimensions: (({ names }: { names: string[] }) => names.map(() => ({ width: 640, height: 480 }))) as never,
    columns_checksum: () => HEX,
    ...answers,
  });
  render(App);
  await backend.ready();
  backend.list('left', '/left', [entry('photo.jpg'), entry('notes.txt'), entry('sub', 'dir')]);
  backend.list('right', '/right', [entry('c.md')]);
  await tick();

  return backend;
}

async function tickBox(name: string) {
  await fireEvent.click(screen.getByRole('button', { name: /^Settings/ }));
  await fireEvent.click(screen.getByRole('menuitem', { name: 'Settings…' }));
  await fireEvent.click(within(settings()).getByRole('checkbox', { name }));
  await fireEvent.click(screen.getByRole('button', { name: 'Close' }));
}

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
});

describe('the Dimensions and Checksum columns', () => {
  it('are not in the panes, and ask for nothing, until the settings show them', async () => {
    const backend = await shown();

    expect(left().queryByRole('button', { name: 'Dimensions' })).not.toBeInTheDocument();
    expect(left().queryByRole('button', { name: 'Checksum' })).not.toBeInTheDocument();
    expect(backend.sent('columns_dimensions')).toEqual([]);
    expect(backend.sent('columns_checksum')).toEqual([]);
  });

  it('show 640×480 for a picture once Dimensions is turned on in the settings, and go when it is turned off', async () => {
    const backend = await shown();

    await tickBox('Dimensions');

    expect(left().getByRole('button', { name: 'Dimensions' })).toBeInTheDocument();
    expect(await left().findByText('640×480')).toHaveAttribute('title', '640 × 480 pixels');
    expect(backend.sent('columns_dimensions')).toEqual([{ path: '/left', names: ['photo.jpg'] }]);
    expect(left().getAllByText('640×480')).toHaveLength(1); // the text file and the folder have none

    await tickBox('Dimensions');

    expect(left().queryByRole('button', { name: 'Dimensions' })).not.toBeInTheDocument();
    expect(left().queryByText('640×480')).not.toBeInTheDocument();
  });

  it('show the first digits of a checksum once Checksum is turned on, with the whole one as the tooltip', async () => {
    const backend = await shown();

    await tickBox('Checksum');

    expect(left().getByRole('button', { name: 'Checksum' })).toBeInTheDocument();
    const cells = await left().findAllByText(HEX.slice(0, 12));
    expect(cells).toHaveLength(2); // the picture and the text file, not the folder
    expect(cells[0]).toHaveAttribute('title', HEX);
    // The setting is for both panes: the right one asks for its own file.
    expect(backend.sent('columns_checksum')).toEqual(
      expect.arrayContaining([
        { path: '/left', name: 'photo.jpg' },
        { path: '/left', name: 'notes.txt' },
        { path: '/right', name: 'c.md' },
      ]),
    );
    expect(backend.sent('columns_checksum')).toHaveLength(3);
  });

  it('come back as the last session left them', async () => {
    const state: SavedState = {
      active: 'left',
      left: { tabs: [{ path: '/left', sort: 'name', desc: false }], active: 0 },
      right: { tabs: [{ path: '/right', sort: 'name', desc: false }], active: 0 },
      hotlist: [],
      preferences: { ...DEFAULT_PREFERENCES, showDimensions: true },
    };
    await shown({ state_load: () => state });

    expect(left().getByRole('button', { name: 'Dimensions' })).toBeInTheDocument();
    expect(await left().findByText('640×480')).toBeInTheDocument();
  });
});

describe('the Git status column', () => {
  const status = (path: string) => ({
    repo: true,
    marks: path === '/left' ? [{ name: 'photo.jpg', mark: 'modified' }, { name: 'sub', mark: 'untracked' }] : [{ name: 'c.md', mark: 'staged' }],
  });

  it('is not in the panes, and asks for nothing, until the settings show it', async () => {
    const backend = await shown({ columns_git: (({ path }: { path: string }) => status(path)) as never });

    expect(left().queryByRole('button', { name: 'Git' })).not.toBeInTheDocument();
    expect(backend.sent('columns_git')).toEqual([]);
  });

  it('shows the marks of each pane once Git status is turned on in the settings, with their words, and goes when it is turned off', async () => {
    const backend = await shown({ columns_git: (({ path }: { path: string }) => status(path)) as never });

    await tickBox('Git status');

    expect(left().getByRole('button', { name: 'Git' })).toBeInTheDocument();
    const modified = await left().findByText('M');
    expect(modified).toHaveAttribute('title', expect.stringMatching(/^Modified/));
    expect(left().getByText('?')).toHaveAttribute('title', expect.stringMatching(/^Untracked/));
    // The setting is for both panes: each asks about its own folder, once.
    expect(await within(pane('Right pane')).findByText('S')).toHaveAttribute('title', expect.stringMatching(/^Staged/));
    expect(backend.sent('columns_git')).toHaveLength(2);
    expect(backend.sent('columns_git')).toEqual(expect.arrayContaining([{ path: '/left' }, { path: '/right' }]));

    await tickBox('Git status');

    expect(left().queryByRole('button', { name: 'Git' })).not.toBeInTheDocument();
    expect(left().queryByText('M')).not.toBeInTheDocument();
  });

  it('is sorted by from its header, the most serious marks first', async () => {
    await shown({ columns_git: (({ path }: { path: string }) => status(path)) as never });
    await tickBox('Git status');
    await left().findByText('M');

    await fireEvent.click(left().getByRole('button', { name: 'Git' }));

    expect(left().getAllByRole('option').map((row) => row.querySelector('.nm')?.textContent)).toEqual(['..', 'sub', 'photo.jpg', 'notes.txt']);
  });

  it('comes back as the last session left it', async () => {
    const state: SavedState = {
      active: 'left',
      left: { tabs: [{ path: '/left', sort: 'name', desc: false }], active: 0 },
      right: { tabs: [{ path: '/right', sort: 'name', desc: false }], active: 0 },
      hotlist: [],
      preferences: { ...DEFAULT_PREFERENCES, showGit: true },
    };
    await shown({ state_load: () => state, columns_git: (({ path }: { path: string }) => status(path)) as never });

    expect(left().getByRole('button', { name: 'Git' })).toBeInTheDocument();
    expect(await left().findByText('M')).toBeInTheDocument();
  });
});
