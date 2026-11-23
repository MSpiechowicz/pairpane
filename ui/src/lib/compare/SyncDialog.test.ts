import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, expect, it } from 'vitest';

import { IpcFailure } from '$lib/ipc/transport';

import { compareBackend, facts, finished, folder, row, rows } from '../../../tests/compare';
import { Syncer } from './syncer.svelte';
import SyncDialog from './SyncDialog.svelte';

const ARCHIVE = { kind: 'archive', archive: '/right/a.zip', dir: 'docs' } as const;

function open(roots: Parameters<Syncer['show']>[0] = { left: folder('/home/ann'), right: folder('/home/bob') }, answers: Parameters<typeof compareBackend>[0] = {}) {
  const backend = compareBackend(answers);
  const syncer = new Syncer(backend.ipc);
  syncer.show(roots, false);
  render(SyncDialog, { syncer, roots });
  return { ...backend, syncer };
}

const dialog = () => screen.getByRole('dialog', { name: 'Synchronize folders' });
const table = () => within(screen.getByRole('table', { name: 'Differences' }));
const button = (name: string | RegExp) => screen.getByRole('button', { name });

/** A scan that has delivered `found` and is done. */
async function scanned(s: ReturnType<typeof open>, ...found: ReturnType<typeof row>[]) {
  await fireEvent.click(button('Start'));
  const scan = s.scans[0];
  scan.emit(rows(...found));
  scan.emit(finished(found.length));
  await tick();
  return scan;
}

describe('SyncDialog', () => {
  it('names both folders (a place in an archive as such) and has the focus on Start', () => {
    open({ left: folder('/home/ann'), right: ARCHIVE });

    expect(dialog()).toBeInTheDocument();
    expect(within(dialog()).getByText('/home/ann')).toBeInTheDocument();
    expect(within(dialog()).getByText('/right/a.zip ▸ docs')).toBeInTheDocument();
    expect(button('Start')).toHaveFocus();
    expect(button('Synchronize')).toBeDisabled();
  });

  it('writes a name that reads as another through displayName', () => {
    open({ left: folder('/home/‮txt.exe'), right: folder('/home/bob') });

    expect(within(dialog()).getByText('/home/⟨U+202E⟩txt.exe')).toBeInTheDocument();
  });

  it('Start scans with the options of the checkboxes', async () => {
    const s = open();
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Subfolders' }));
    await fireEvent.click(screen.getByRole('checkbox', { name: 'By content' }));
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Hidden files' }));

    await fireEvent.click(button('Start'));

    expect(s.scans[0].args.options).toEqual({ subfolders: false, content: true, hidden: true });
    expect(button('Stop')).toBeInTheDocument();
  });

  it('turns content off with a note when a side is an archive', () => {
    open({ left: folder('/home/ann'), right: ARCHIVE });

    expect(screen.getByRole('checkbox', { name: 'By content' })).toBeDisabled();
    expect(within(dialog()).getByText('Files in an archive are not compared by content.')).toBeInTheDocument();
  });

  it('shows the rows with their state in words, what each side holds, and the action chosen', async () => {
    const s = open();
    await scanned(s, row('docs/a.txt', 'leftOnly', facts({ size: 2048 }), null), row('b.txt', 'rightNewer'), row('c.txt', 'differs'));

    const rowsShown = table().getAllByRole('row').slice(1);

    expect(rowsShown.map((item) => item.textContent?.replace(/\s+/g, ' ').trim())).toEqual([
      expect.stringContaining('docs/a.txt'),
      expect.stringContaining('b.txt'),
      expect.stringContaining('c.txt'),
    ]);
    expect(within(rowsShown[0]).getByText('Only left')).toBeInTheDocument();
    expect(within(rowsShown[0]).getByText(/^2\.0 KB · 2026-11-20/)).toBeInTheDocument();
    expect(within(rowsShown[1]).getByText('Right newer')).toBeInTheDocument();
    expect(within(rowsShown[0]).getByRole('button', { name: 'Copy to the right: docs/a.txt' })).toHaveAttribute('aria-pressed', 'true');
    expect(within(rowsShown[1]).getByRole('button', { name: 'Copy to the left: b.txt' })).toHaveAttribute('aria-pressed', 'true');
    expect(within(rowsShown[2]).getByRole('button', { name: 'Skip: c.txt' })).toHaveAttribute('aria-pressed', 'true');
  });

  it('writes the path of a row through displayName', async () => {
    const s = open();
    await scanned(s, row('‮txt.exe', 'leftOnly', facts(), null));

    expect(table().getByText('⟨U+202E⟩txt.exe')).toBeInTheDocument();
  });

  it('disables the arrows a row cannot take, and says why in the title', async () => {
    const s = open({ left: folder('/home/ann'), right: ARCHIVE });
    await scanned(
      s,
      row('lo.txt', 'leftOnly', facts(), null),
      row('link', 'leftOnly', facts({ link: true, kind: 'other' }), null),
      row('mixed', 'differs', facts({ kind: 'dir' }), facts()),
      row('ro.txt', 'rightOnly', null, facts()),
    );

    const [only, link, mixed, from] = table().getAllByRole('row').slice(1);

    const intoArchive = within(only).getByRole('button', { name: 'Copy to the right: lo.txt' });
    expect(intoArchive).toBeDisabled();
    expect(intoArchive).toHaveAttribute('title', 'Archives can only be copied from');
    expect(within(only).getByRole('button', { name: 'Skip: lo.txt' })).toHaveAttribute('aria-pressed', 'true');
    expect(within(link).getByRole('button', { name: 'Copy to the right: link' })).toHaveAttribute('title', 'Links are not copied.');
    expect(within(mixed).getByRole('button', { name: 'Copy to the left: mixed' })).toHaveAttribute('title', 'A file and a folder cannot replace each other.');
    expect(within(from).getByRole('button', { name: 'Copy to the left: ro.txt' })).toBeEnabled();
  });

  it('a click on an arrow changes the action of the row, and the totals follow', async () => {
    const s = open();
    await scanned(s, row('a.txt', 'leftNewer', facts({ size: 1024 }), facts({ size: 5 })));
    expect(screen.getByText(/→ 1 item · 1\.0 KB/)).toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: 'Skip: a.txt' }));

    expect(screen.getByText('Nothing is to be copied or removed.')).toBeInTheDocument();
    expect(button('Synchronize')).toBeDisabled();
  });

  it('picks rows and sets their action together', async () => {
    const s = open();
    await scanned(s, row('a.txt', 'leftNewer'), row('b.txt', 'leftNewer'), row('c.txt', 'leftNewer'));
    expect(button('Skip picked rows')).toBeDisabled();

    await fireEvent.click(screen.getByRole('checkbox', { name: 'Pick a.txt' }));
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Pick c.txt' }));
    expect(screen.getByText('Picked: 2')).toBeInTheDocument();
    await fireEvent.click(button('Copy picked rows to the left'));

    const pressed = (name: string) => screen.getByRole('button', { name }).getAttribute('aria-pressed');
    expect([pressed('Copy to the left: a.txt'), pressed('Copy to the right: b.txt'), pressed('Copy to the left: c.txt')]).toEqual(['true', 'true', 'true']);

    await fireEvent.click(screen.getByRole('checkbox', { name: 'Pick every row shown' }));
    expect(screen.getByText('Picked: 3')).toBeInTheDocument();
  });

  it('hides the statuses that are unchecked, and the names the name filter does not match', async () => {
    const s = open();
    await scanned(s, row('a.txt', 'leftNewer'), row('b.md', 'leftNewer'), row('c.txt', 'equal'));
    const names = () => table().queryAllByRole('row').slice(1).map((item) => item.textContent);
    expect(names()).toHaveLength(2);

    await fireEvent.click(screen.getByRole('checkbox', { name: 'Equal (1)' }));
    expect(names()).toHaveLength(3);

    await fireEvent.input(screen.getByRole('textbox', { name: 'Names' }), { target: { value: '*.txt' } });
    expect(names()).toEqual([expect.stringContaining('a.txt'), expect.stringContaining('c.txt')]);
  });

  it('after an option changes, says the rows are old and does not synchronize until Start', async () => {
    const s = open();
    await scanned(s, row('a.txt', 'leftNewer'));
    expect(button('Synchronize')).toBeEnabled();
    expect(screen.queryByText('The options changed: press Start to compare again.')).not.toBeInTheDocument();

    await fireEvent.click(screen.getByRole('checkbox', { name: 'Hidden files' }));

    expect(screen.getByText('The options changed: press Start to compare again.')).toHaveAttribute('role', 'status');
    expect(button('Synchronize')).toBeDisabled();
    expect(table().queryAllByRole('row')).toHaveLength(2);

    await fireEvent.click(button('Start'));

    expect(screen.queryByText('The options changed: press Start to compare again.')).not.toBeInTheDocument();
    expect(s.scans[1].args.options.hidden).toBe(true);
  });

  it('says how the scan went, and what it could not do', async () => {
    const s = open();
    await fireEvent.click(button('Start'));
    s.scans[0].emit({ kind: 'progress', folders: 3, files: 10, bytes: 2048 });
    await tick();
    expect(screen.getByText('Comparing… 3 folders · 10 files · 2.0 KB')).toBeInTheDocument();

    s.scans[0].emit(finished(4, { skipped: 2, truncated: true }));
    await tick();
    expect(screen.getByText('4 rows · 2 skipped (unreadable, unusable or alike names) · the scan stopped at its limit: narrow it to see more')).toBeInTheDocument();
  });

  it('shows an error of the scan, as it is said', async () => {
    const s = open();
    await fireEvent.click(button('Start'));

    s.scans[0].emit({ kind: 'error', error: { code: 'invalidInput', message: 'One folder is inside the other' } });
    await tick();

    expect(screen.getByRole('alert')).toHaveTextContent('One folder is inside the other');
  });

  it('Stop asks the backend to stop, and Close drops the scan', async () => {
    const s = open();
    await fireEvent.click(button('Start'));

    await fireEvent.click(button('Stop'));
    expect(s.calls('compare_cancel')).toEqual([{ id: 1 }]);

    await fireEvent.click(button('Close'));
    expect(s.calls('compare_forget')).toEqual([{ id: 1 }]);
    expect(s.syncer.modal).toBe(false);
  });
});

describe('SyncDialog confirmation', () => {
  async function ready(answers: Parameters<typeof compareBackend>[0] = {}) {
    const s = open(undefined, answers);
    await scanned(
      s,
      row('new.txt', 'leftOnly', facts({ size: 1024 }), null),
      row('over.txt', 'leftNewer', facts({ size: 1024 }), facts({ size: 5 })),
      row('back.txt', 'rightNewer', facts({ size: 5 }), facts({ size: 2048 })),
    );
    await fireEvent.click(button('Synchronize'));
    return s;
  }

  const confirm = () => screen.getByRole('dialog', { name: 'Synchronize 3 items?' });

  it('asks first, saying how many files it overwrites and that nothing is deleted', async () => {
    const s = await ready();

    expect(within(confirm()).getByText('Copy 2 items (2.0 KB) to the right and 1 item (2.0 KB) to the left.')).toBeInTheDocument();
    expect(within(confirm()).getByText(/2 files will be overwritten\. This cannot be undone\./)).toBeInTheDocument();
    expect(within(confirm()).getByText(/Nothing is deleted\./)).toBeInTheDocument();
    expect(within(confirm()).getByRole('button', { name: 'Cancel' })).toHaveFocus();
    expect(s.calls('ops_sync')).toEqual([]);
  });

  it('says how many rows with an action the filters hide, and sends only the rows shown', async () => {
    const s = await ready();
    await fireEvent.click(within(confirm()).getByRole('button', { name: 'Cancel' }));
    await fireEvent.input(screen.getByRole('textbox', { name: 'Names' }), { target: { value: 'new.*; back.*' } });

    await fireEvent.click(button('Synchronize'));

    const asked = screen.getByRole('dialog', { name: 'Synchronize 2 items?' });
    expect(within(asked).getByText('1 row with an action is hidden by the filters and will not be synchronized.')).toBeInTheDocument();
    await fireEvent.click(within(asked).getByRole('button', { name: 'Synchronize' }));

    await waitFor(() => expect(s.calls('ops_sync')).toHaveLength(1));
    expect(s.calls('ops_sync')[0]).toMatchObject({ items: [{ path: 'new.txt' }, { path: 'back.txt' }] });
  });

  it('says nothing of hidden rows when the filters hide none', async () => {
    await ready();

    expect(within(confirm()).queryByText(/hidden by the filters/)).not.toBeInTheDocument();
  });

  it('Cancel goes back to the table', async () => {
    const s = await ready();

    await fireEvent.click(within(confirm()).getByRole('button', { name: 'Cancel' }));

    expect(screen.queryByRole('dialog', { name: 'Synchronize 3 items?' })).not.toBeInTheDocument();
    expect(dialog()).toBeInTheDocument();
    expect(s.calls('ops_sync')).toEqual([]);
  });

  it('Synchronize queues the one job and closes the dialogs', async () => {
    const s = await ready();

    await fireEvent.click(within(confirm()).getByRole('button', { name: 'Synchronize' }));

    await waitFor(() => expect(s.calls('ops_sync')).toHaveLength(1));
    expect(s.calls('ops_sync')[0]).toMatchObject({
      left: folder('/home/ann'),
      right: folder('/home/bob'),
      items: [
        { path: 'new.txt', direction: 'toRight', target: null },
        { path: 'over.txt', direction: 'toRight', target: { kind: 'file', size: 5 } },
        { path: 'back.txt', direction: 'toLeft', source: { kind: 'file', size: 2048 } },
      ],
    });
    // The window unmounts the dialog when it is closed: here the dialog's state says it is.
    await waitFor(() => expect(s.syncer.modal).toBe(false));
    expect(screen.queryByRole('dialog', { name: 'Synchronize 3 items?' })).not.toBeInTheDocument();
  });

  it('a refusal is shown in the confirmation, which stays', async () => {
    await ready({
      ops_sync: () => {
        throw new IpcFailure({ code: 'invalidInput', message: 'too many items' });
      },
    });

    await fireEvent.click(within(confirm()).getByRole('button', { name: 'Synchronize' }));

    expect(await within(confirm()).findByRole('alert')).toHaveTextContent('too many items');
    expect(dialog()).toBeInTheDocument();
  });

  it('needs no warning when nothing is overwritten', async () => {
    const s = open();
    await scanned(s, row('new.txt', 'leftOnly', facts(), null));

    await fireEvent.click(button('Synchronize'));

    const asked = screen.getByRole('dialog', { name: 'Synchronize 1 item?' });
    expect(within(asked).getByText(/Nothing will be overwritten\./)).toBeInTheDocument();
    expect(within(asked).getByRole('button', { name: 'Synchronize' })).toHaveFocus();
  });

  describe('a mirror', () => {
    async function mirrored() {
      const s = open();
      await scanned(
        s,
        row('new.txt', 'leftOnly', facts({ size: 10 }), null),
        row('old.txt', 'rightOnly', null, facts({ size: 20 })),
        row('old-dir', 'rightOnly', null, facts({ kind: 'dir', size: 0, files: 2, bytes: 30 })),
      );
      await fireEvent.click(button('Mirror →'));
      await fireEvent.click(button('Synchronize'));
      return s;
    }

    const asked = () => screen.getByRole('dialog', { name: 'Synchronize 3 items?' });

    it('lists every item it removes, says how many, and sends them to the Trash', async () => {
      const s = await mirrored();

      expect(within(asked()).getByText(/Remove 2 items \(50 B\) from the right, to the Trash\./)).toBeInTheDocument();
      const listed = within(asked()).getByRole('list', { name: 'Items to remove' });
      expect(within(listed).getAllByRole('listitem').map((item) => item.textContent?.replace(/\s+/g, ' ').trim())).toEqual([
        'Right old.txt',
        'Right old-dir',
      ]);
      expect(within(asked()).getByRole('button', { name: 'Cancel' })).toHaveFocus();

      await fireEvent.click(within(asked()).getByRole('button', { name: 'Synchronize' }));

      await waitFor(() => expect(s.calls('ops_sync')).toHaveLength(1));
      expect(s.calls('ops_sync')[0]).toMatchObject({
        items: [{ path: 'new.txt', direction: 'toRight' }],
        removals: [
          { path: 'old.txt', side: 'right' },
          { path: 'old-dir', side: 'right', files: 2, bytes: 30 },
        ],
        permanent: false,
      });
    });

    it('deletes for good only after a second confirmation of its own; Back returns to the first', async () => {
      const s = await mirrored();
      await fireEvent.click(within(asked()).getByRole('checkbox', { name: 'Delete permanently instead of moving to the Trash' }));

      await fireEvent.click(within(asked()).getByRole('button', { name: 'Continue…' }));

      const second = screen.getByRole('dialog', { name: 'Delete 2 items permanently?' });
      expect(within(second).getByText(/do not go to the Trash and cannot be restored/)).toBeInTheDocument();
      expect(within(second).getByRole('button', { name: 'Back' })).toHaveFocus();
      expect(s.calls('ops_sync')).toEqual([]);

      await fireEvent.click(within(second).getByRole('button', { name: 'Back' }));
      await fireEvent.click(within(asked()).getByRole('button', { name: 'Continue…' }));
      await fireEvent.click(within(screen.getByRole('dialog', { name: 'Delete 2 items permanently?' })).getByRole('button', { name: 'Delete permanently' }));

      await waitFor(() => expect(s.calls('ops_sync')).toHaveLength(1));
      expect(s.calls('ops_sync')[0]).toMatchObject({ permanent: true });
    });

    it('a row can be set to removal on its own, and the totals say so', async () => {
      const s = open();
      await scanned(s, row('old.txt', 'rightOnly', null, facts({ size: 20 })));

      await fireEvent.click(screen.getByRole('button', { name: 'Remove: old.txt' }));

      expect(screen.getByText(/✕ 1 item · 20 B/)).toBeInTheDocument();
      expect(button('Synchronize')).toBeEnabled();
    });
  });
});
