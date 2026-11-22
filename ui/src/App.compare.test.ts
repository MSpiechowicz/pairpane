import { fireEvent, screen, waitFor, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import { type Answers, appShown, key, listbox, withArchiveOnRight } from '../tests/app';
import { entry } from '../tests/panes';

const T = Date.UTC(2026, 10, 20, 12, 0);

const compare = () => fireEvent.click(screen.getByRole('button', { name: '⇆ Compare' }));
const notice = () => screen.findByRole('region', { name: 'Notice' });
const option = (side: 'Left' | 'Right', name: RegExp) => within(listbox(side)).getByRole('option', { name });
const dialog = () => screen.getByRole('dialog', { name: 'Synchronize folders' });

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
});

describe('Compare folders', () => {
  it('marks what only one pane has, selects it, and counts it in a card', async () => {
    await appShown(invoke);

    await compare();

    expect(await notice()).toHaveTextContent('3 only on the left, 1 only on the right.');
    for (const name of [/a pdf/, /b txt/, /sub/]) {
      expect(option('Left', name)).toHaveAttribute('title', 'Only in this pane');
      expect(option('Left', name)).toHaveAttribute('aria-selected', 'true');
    }

    expect(option('Right', /c md/)).toHaveAttribute('title', 'Only in this pane');
    expect(option('Right', /c md/)).toHaveAttribute('aria-selected', 'true');
  });

  it('Files, Compare folders does the same', async () => {
    await appShown(invoke);

    await fireEvent.click(screen.getByRole('button', { name: 'Files' }));
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Compare folders' }));

    expect(await notice()).toHaveTextContent('3 only on the left, 1 only on the right.');
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('marks the newer and the older file, selects only the newer, and leaves equal ones and folders alone', async () => {
    const backend = await appShown(invoke);
    backend.list('left', '/left', [
      entry('new.txt', 'file', { modified: T + 60_000, size: 5 }),
      entry('old.txt', 'file', { modified: T, size: 5 }),
      entry('same.txt', 'file', { modified: T + 1000, size: 5 }),
      entry('dir', 'dir', { modified: T }),
    ]);
    backend.list('right', '/right', [
      entry('new.txt', 'file', { modified: T, size: 5 }),
      entry('old.txt', 'file', { modified: T + 60_000, size: 5 }),
      entry('same.txt', 'file', { modified: T, size: 5 }),
      entry('dir', 'dir', { modified: T + 99_000 }),
    ]);
    await tick();

    await compare();

    expect(await notice()).toHaveTextContent('1 newer on the left, 1 newer on the right.');
    expect(option('Left', /new/)).toHaveAttribute('title', 'Newer than in the other pane');
    expect(option('Left', /new/)).toHaveAttribute('aria-selected', 'true');
    expect(option('Left', /old/)).toHaveAttribute('title', 'Older than in the other pane');
    expect(option('Left', /old/)).toHaveAttribute('aria-selected', 'false');
    expect(option('Right', /old/)).toHaveAttribute('aria-selected', 'true');
    expect(option('Left', /same/)).not.toHaveAttribute('title');
    expect(option('Left', /dir/)).not.toHaveAttribute('title');
  });

  it('marks and selects the rows of an archive pane too', async () => {
    await withArchiveOnRight(invoke, 'd.zip');

    await compare();

    expect(await notice()).toHaveTextContent('3 only on the left, 3 only on the right.');
    expect(option('Right', /readme/)).toHaveAttribute('title', 'Only in this pane');
    expect(option('Right', /readme/)).toHaveAttribute('aria-selected', 'true');
  });

  it('the marks are gone when a pane lists again, a reload included', async () => {
    const backend = await appShown(invoke);
    await compare();
    expect(option('Left', /a pdf/)).toHaveAttribute('title');

    backend.list('left', '/left', [entry('a.pdf'), entry('b.txt'), entry('sub', 'dir')]);
    await tick();

    expect(option('Left', /a pdf/)).not.toHaveAttribute('title');
    expect(option('Right', /c md/)).toHaveAttribute('title', 'Only in this pane');

    backend.list('right', '/elsewhere', [entry('c.md')]);
    await tick();

    expect(option('Right', /c md/)).not.toHaveAttribute('title');
  });

  it('says so when the folders match', async () => {
    const backend = await appShown(invoke);
    backend.list('left', '/left', [entry('x', 'file', { modified: T, size: 1 })]);
    backend.list('right', '/right', [entry('x', 'file', { modified: T + 500, size: 1 })]);
    await tick();

    await compare();

    expect(await notice()).toHaveTextContent('The folders match.');
  });

  it('is refused when both panes show the same folder', async () => {
    const backend = await appShown(invoke);
    backend.list('right', '/left', [entry('a.pdf')]);
    await tick();

    await compare();

    expect(await notice()).toHaveTextContent('Both panes show the same folder.');
    expect(option('Left', /a pdf/)).not.toHaveAttribute('title');
  });
});

describe('Synchronize folders', () => {
  it('opens on the two folders, from the Files menu and from the Sync button', async () => {
    await appShown(invoke);

    await fireEvent.click(screen.getByRole('button', { name: 'Files' }));
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Synchronize folders…' }));

    expect(within(dialog()).getByText('/left')).toBeInTheDocument();
    expect(within(dialog()).getByText('/right')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Start' })).toHaveFocus();
    await fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: '⇅ Sync' }));
    expect(dialog()).toBeInTheDocument();
  });

  it('names a folder inside an archive as the archive and the folder in it, and scans with it', async () => {
    const scans: { args: unknown }[] = [];
    const answers: Answers = { compare_start: (args) => scans.push({ args }) };
    await withArchiveOnRight(invoke, 'd.zip', answers);

    await fireEvent.click(screen.getByRole('button', { name: '⇅ Sync' }));
    expect(within(dialog()).getByText('/right/d.zip ▸ /')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Start' }));

    await waitFor(() => expect(scans).toHaveLength(1));
    expect(scans[0].args).toMatchObject({
      left: { kind: 'folder', path: '/left' },
      right: { kind: 'archive', archive: '/right/d.zip', dir: '' },
      options: { subfolders: true, content: false, hidden: false },
    });
  });

  it('scans hidden files when the panes list them', async () => {
    const backend = await appShown(invoke);
    await fireEvent.click(screen.getByRole('button', { name: 'Hidden' }));
    expect(backend.sent('pane_open').at(-1)).toMatchObject({ showHidden: true });
    backend.list('left', '/left', [entry('a.pdf')]);
    backend.list('right', '/right', [entry('c.md')]);
    await tick();

    await fireEvent.click(screen.getByRole('button', { name: '⇅ Sync' }));

    expect(screen.getByRole('checkbox', { name: 'Hidden files' })).toBeChecked();
  });

  it('waits for a pane that is still listing', async () => {
    const backend = await appShown(invoke);
    await fireEvent.click(screen.getByRole('button', { name: 'Hidden' })); // both panes list again
    expect(backend.sent('pane_open').at(-1)).toMatchObject({ showHidden: true });

    await fireEvent.click(screen.getByRole('button', { name: '⇅ Sync' }));
    expect(await notice()).toHaveTextContent('Wait until the folders have been listed.');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();

    await compare();
    expect(await notice()).toHaveTextContent('Wait until the folders have been listed.');
  });

  it('has the keyboard while it is open, and Escape closes it and drops the scan', async () => {
    const backend = await appShown(invoke, { compare_start: () => 7 });
    await fireEvent.click(screen.getByRole('button', { name: '⇅ Sync' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Start' }));
    await waitFor(() => expect(backend.sent('compare_start')).toHaveLength(1));

    await key(window, 'F7');
    expect(screen.queryByRole('dialog', { name: 'New folder' })).not.toBeInTheDocument();
    expect(backend.sent('ops_mkdir')).toEqual([]);

    await key(window, 'Escape');

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    await waitFor(() => expect(backend.sent('compare_forget')).toEqual([{ id: 7 }]));
    expect(backend.sent('compare_cancel')).toEqual([{ id: 7 }]);
  });

  it('is refused when both panes show the same folder', async () => {
    const backend = await appShown(invoke);
    backend.list('right', '/left', [entry('a.pdf')]);
    await tick();

    await fireEvent.click(screen.getByRole('button', { name: '⇅ Sync' }));

    expect(await notice()).toHaveTextContent('Both panes show the same folder.');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});
