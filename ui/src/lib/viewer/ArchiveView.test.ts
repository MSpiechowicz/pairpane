import { render, screen, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';

import type { ArchiveListing } from '$lib/ipc/bindings/ArchiveListing';
import { formatDate } from '$lib/panes/format';

import { FakeFile, openedSession } from '../../../tests/view';
import ArchiveView from './ArchiveView.svelte';

const MODIFIED = Date.UTC(2026, 10, 20, 7, 58);

const listing = (extra: Partial<ArchiveListing> = {}): ArchiveListing => ({
  format: 'zip',
  truncated: false,
  entries: [
    { name: 'docs/', size: 0, dir: true, modified: null },
    { name: 'docs/readme.txt', size: 1536, dir: false, modified: MODIFIED },
    { name: 'a.bin', size: 98, dir: false, modified: null },
  ],
  ...extra,
});

async function open(answer: ArchiveListing | (() => Promise<ArchiveListing>)) {
  const world = await openedSession(new FakeFile('PK'), () => ({ kind: 'archive', format: 'zip' }), {
    view_archive: () => (typeof answer === 'function' ? answer() : answer),
  });
  render(ArchiveView, { session: world.session });

  return world;
}

describe('ArchiveView', () => {
  it('lists the entries with name, size and date, and marks the folders', async () => {
    const { sent } = await open(listing());

    const rows = await screen.findAllByRole('row');
    const cells = rows.map((row) => within(row).queryAllByRole('cell').map((cell) => cell.textContent));

    expect(sent('view_archive')).toEqual([{ session: 1 }]);
    expect(screen.getAllByRole('columnheader').map((cell) => cell.textContent)).toEqual(['Name', 'Size', 'Modified']);
    expect(cells.slice(1)).toEqual([
      ['docs/', 'Folder', ''],
      ['docs/readme.txt', '1.5 KB', formatDate(MODIFIED)],
      ['a.bin', '98 B', ''],
    ]);
    expect(rows[1]).toHaveClass('dir');
    expect(rows[2]).not.toHaveClass('dir');
  });

  it('marks a folder whose name has no slash', async () => {
    await open(listing({ entries: [{ name: 'src', size: 0, dir: true, modified: null }] }));

    expect(await screen.findByText('src/')).toBeInTheDocument();
  });

  it('says when the listing is cut short', async () => {
    await open(listing({ truncated: true }));

    expect(await screen.findByText('Only the first entries are listed')).toBeInTheDocument();
  });

  it('says when there is nothing in it', async () => {
    await open(listing({ entries: [] }));

    expect(await screen.findByText('The archive is empty.')).toBeInTheDocument();
  });

  it('shows the backend`s words when the archive cannot be read', async () => {
    await open(() => Promise.reject({ code: 'io', message: 'The archive is damaged' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('The archive is damaged');
  });
});
