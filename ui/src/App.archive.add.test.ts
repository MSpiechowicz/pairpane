import { fireEvent, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import { IpcFailure } from '$lib/ipc/transport';

import { key, listbox, withArchiveOnRight } from '../tests/app';
import { entry } from '../tests/panes';

const ZIP = { archive: '/left/a.zip', dir: '' };
const CANT_ADD = 'Files can’t be added to this kind of archive.';
const EXTRACT_INTO = 'Can’t extract into an archive.';

const needsPassword = (message = 'The archive needs a password.') => {
  throw new IpcFailure({ code: 'passwordRequired', message });
};

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
});

describe('copying into an archive', () => {
  it.each(['a.zip', 'a.jar', 'a.tar', 'a.tar.gz', 'a.tgz', 'a.tar.xz', 'a.txz', 'a.tar.zst', 'a.tzst', 'a.tar.bz2', 'a.tbz2', 'a.tbz', 'a.7z'])(
    'F5 into %s asks with the archive fixed, and adds the item under the cursor to it',
    async (name) => {
      const backend = await withArchiveOnRight(invoke, name, { ops_add_to_archive: () => 4 });

      await key(listbox('Left'), 'F5');
      const field = screen.getByRole('textbox', { name: 'Archive folder' });
      expect(screen.getByRole('dialog', { name: 'Copy “a.zip” to' })).toBeInTheDocument();
      expect(field).toHaveValue(`/right/${name} ▸ /`);
      expect(field).toHaveAttribute('readonly');
      await key(field, 'Enter');

      await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
      expect(backend.sent('ops_add_to_archive')).toEqual([{ archive: `/right/${name}`, dir: '', from: '/left', names: ['a.zip'] }]);
      expect(backend.sent('ops_start')).toEqual([]);
    },
  );

  it('adds the selection into the folder of the archive the pane shows', async () => {
    const backend = await withArchiveOnRight(invoke, 'd.7z', { ops_add_to_archive: () => 4 }, 'docs/img');
    await key(listbox('Left'), 'Insert'); // a.zip
    await key(listbox('Left'), 'Insert'); // b.txt

    await key(listbox('Left'), 'F5');
    expect(screen.getByRole('dialog', { name: 'Copy 2 items to' })).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Archive folder' })).toHaveValue('/right/d.7z ▸ docs/img');
    await key(screen.getByRole('textbox', { name: 'Archive folder' }), 'Enter');

    await vi.waitFor(() => expect(backend.sent('ops_add_to_archive')).toHaveLength(1));
    expect(backend.sent('ops_add_to_archive')[0]).toEqual({ archive: '/right/d.7z', dir: 'docs/img', from: '/left', names: ['a.zip', 'b.txt'] });
  });

  it.each(['d.rar', 'notes.gz'])('F5 into %s says the archive cannot take files, and opens nothing', async (name) => {
    const backend = await withArchiveOnRight(invoke, name);

    await key(listbox('Left'), 'F5');

    expect(await screen.findByText(CANT_ADD)).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(backend.sent('ops_add_to_archive')).toEqual([]);
  });

  it('F5 from an archive pane into another archive pane is refused: that would extract into one', async () => {
    const backend = await withArchiveOnRight(invoke, 'd.zip');
    await key(listbox('Left'), 'Enter'); // a.zip
    await vi.waitFor(() => expect(backend.sent('archive_open')).toHaveLength(2));
    backend.listIn('left', ZIP, [entry('docs', 'dir'), entry('readme.txt')]);
    await tick();
    await key(listbox('Left'), 'ArrowDown');
    await key(listbox('Left'), 'ArrowDown');

    await key(listbox('Left'), 'F5');

    expect(await screen.findByText(EXTRACT_INTO)).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(backend.sent('ops_extract')).toEqual([]);
    expect(backend.sent('ops_add_to_archive')).toEqual([]);
  });

  it('asks for the password of an encrypted archive, then adds once the right one was given', async () => {
    let tries = 0;
    const backend = await withArchiveOnRight(invoke, 'd.7z', {
      ops_add_to_archive: () => {
        tries += 1;
        return tries === 1 ? needsPassword() : 4;
      },
      archive_password: () => true,
    });
    await key(listbox('Left'), 'F5');
    await key(screen.getByRole('textbox', { name: 'Archive folder' }), 'Enter');

    const field = await screen.findByLabelText('Password');
    expect(screen.getByRole('dialog', { name: 'Password for “d.7z”' })).toBeInTheDocument();
    await fireEvent.input(field, { target: { value: 'secret' } });
    await key(field, 'Enter');

    await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(backend.sent('archive_password')).toEqual([{ archive: '/right/d.7z', password: 'secret' }]);
    expect(backend.sent('ops_add_to_archive')).toHaveLength(2);
  });
});

describe('an archive pane after an add to its archive', () => {
  const job = (id: number, kind: 'addToArchive' | 'copy', to: string) => ({
    id,
    kind,
    label: 'a.zip',
    from: '/left',
    to,
    items: 1,
    verify: false,
    phase: 'copying' as const,
    paused: false,
    doneBytes: 0,
    totalBytes: 0,
    doneFiles: 0,
    totalFiles: 0,
    current: null,
    bytesPerSec: 0,
    etaSecs: null,
  });
  const ended = (id: number) => ({ kind: 'ended' as const, id, outcome: { kind: 'done' as const }, skipped: 0, errors: [], moreErrors: 0 });

  it('lists the archive again, in the folder it was in, with the cursor kept; the pane that shows another archive is left alone', async () => {
    const backend = await withArchiveOnRight(invoke, 'd.zip', {}, 'docs');
    await key(listbox('Left'), 'Enter'); // a.zip, in the left pane
    await vi.waitFor(() => expect(backend.sent('archive_open')).toHaveLength(2));
    backend.listIn('left', ZIP, [entry('docs', 'dir'), entry('readme.txt')]);
    await tick();
    await key(listbox('Right'), 'ArrowDown');
    await key(listbox('Right'), 'ArrowDown');
    const opens = () => backend.sent('archive_open').length;

    backend.ops(
      { kind: 'added', job: job(1, 'copy', '/right/d.zip') },
      { kind: 'added', job: job(2, 'addToArchive', '/elsewhere/x.zip') },
      ended(1),
      ended(2),
    );
    await tick();
    expect(opens()).toBe(2);

    backend.ops({ kind: 'added', job: job(3, 'addToArchive', '/right/d.zip') }, ended(3));
    await vi.waitFor(() => expect(opens()).toBe(3));
    expect(backend.sent('archive_open')[2]).toMatchObject({ pane: 'right', archive: '/right/d.zip', dir: 'docs' });

    backend.listIn('right', { archive: '/right/d.zip', dir: 'docs' }, [entry('aaa.txt'), entry('docs', 'dir'), entry('readme.txt'), entry('notes.txt')]);
    await tick();
    const row = within(listbox('Right'))
      .getAllByRole('option')
      .find((option) => option.classList.contains('cursor'));
    expect(row).toHaveTextContent('readme');
  });
});
