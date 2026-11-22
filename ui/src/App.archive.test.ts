import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import { IpcFailure } from '$lib/ipc/transport';

import App from './App.svelte';

import { type Answers, appWorld, key, listbox } from '../tests/app';
import { entry } from '../tests/panes';
import { FakeFile, numbered, viewAnswers } from '../tests/view';

const READ_ONLY = 'Archives are read-only here — extract first (F5).';
const ZIP = { archive: '/left/a.zip', dir: '' };

const needsPassword = (message = 'The archive needs a password.') => {
  throw new IpcFailure({ code: 'passwordRequired', message });
};

/**
 * Both panes listed and the left cursor on `a.zip`; with `open`, the left pane is inside that archive (rows `..`,
 * docs, readme.txt) and the right one shows `/right` (`c.md`, `d.tar.gz`, `e.txt`).
 */
async function world(answers: Answers = {}, open = true) {
  const backend = appWorld(invoke, answers);
  render(App);
  await backend.ready();
  backend.list('left', '/left', [entry('a.zip'), entry('b.txt'), entry('f.7z')]);
  backend.list('right', '/right', [entry('c.md'), entry('d.tar.gz'), entry('e.txt')]);
  await tick();
  await key(listbox('Left'), 'ArrowDown');

  if (open) {
    await key(listbox('Left'), 'Enter');
    await vi.waitFor(() => expect(backend.sent('archive_open')).toHaveLength(1));
    backend.listIn('left', ZIP, [entry('docs', 'dir'), entry('readme.txt'), entry('notes.txt')]);
    await tick();
  }

  return backend;
}

/** The cursor of the left pane on its row `row` (`..` is row 0). */
async function cursorTo(row: number) {
  for (let step = 0; step < row; step += 1) {
    await key(listbox('Left'), 'ArrowDown');
  }
}

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
});

describe('an archive is read-only', () => {
  it.each([
    ['F6', {}],
    ['F6', { shiftKey: true }],
    ['F2', {}],
    ['F7', {}],
    ['F4', { shiftKey: true }],
    ['F4', {}],
    ['F8', {}],
    ['F8', { shiftKey: true }],
    ['Delete', {}],
    ['Delete', { shiftKey: true }],
    ['Enter', { altKey: true }],
  ])('%s %o says so and does nothing', async (name, init) => {
    const backend = await world();
    await cursorTo(2);

    await key(listbox('Left'), name, init);

    expect(await screen.findByText(READ_ONLY)).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    for (const command of ['ops_start', 'ops_extract', 'ops_delete', 'ops_trash', 'ops_rename', 'ops_mkdir', 'ops_mkfile', 'edit_external']) {
      expect(backend.sent(command)).toEqual([]);
    }
  });

  it('the Files menu items that change something say so too', async () => {
    await world();
    await cursorTo(2);

    await fireEvent.click(screen.getByRole('button', { name: 'Files' }));
    await fireEvent.click(await screen.findByRole('menuitem', { name: 'Create symbolic link…' }));

    expect(await screen.findByText(READ_ONLY)).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('F5 and F6 into a pane that shows an archive are refused', async () => {
    const backend = await world({}, false);
    await key(document.body, 'Tab');
    await key(listbox('Right'), 'ArrowDown');
    await key(listbox('Right'), 'ArrowDown'); // d.tar.gz
    await key(listbox('Right'), 'Enter');
    await vi.waitFor(() => expect(backend.sent('archive_open')).toHaveLength(1));
    backend.listIn('right', { archive: '/right/d.tar.gz', dir: '' }, [entry('x.txt')]);
    await tick();
    await key(document.body, 'Tab');

    for (const name of ['F5', 'F6']) {
      await key(listbox('Left'), name);
      expect(await screen.findByText('Can’t copy into an archive.')).toBeInTheDocument();
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    }
    expect(backend.sent('ops_start')).toEqual([]);
  });
});

describe('extracting', () => {
  it('F5 in an archive asks where, with the other pane`s folder, and extracts what the cursor is on', async () => {
    const backend = await world({ ops_extract: () => 3 });
    await cursorTo(2);

    await key(listbox('Left'), 'F5');
    const field = screen.getByRole('textbox', { name: 'Target folder' });
    expect(screen.getByRole('dialog', { name: 'Extract “readme.txt” to' })).toBeInTheDocument();
    expect(field).toHaveValue('/right');
    await key(field, 'Enter');

    await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(backend.sent('ops_extract')).toEqual([{ archive: '/left/a.zip', dir: '', names: ['readme.txt'], to: '/right' }]);
    expect(backend.sent('ops_start')).toEqual([]);
  });

  it('extracts the selection, folders too, from the folder shown', async () => {
    const backend = await world({ ops_extract: () => 3 });
    await key(listbox('Left'), 'Insert'); // `..` cannot be selected: the cursor moves on to docs
    await key(listbox('Left'), 'Insert');
    await key(listbox('Left'), 'Insert');

    await key(listbox('Left'), 'F5');
    expect(screen.getByRole('dialog', { name: 'Extract 2 items to' })).toBeInTheDocument();
    await key(screen.getByRole('textbox', { name: 'Target folder' }), 'Enter');

    await vi.waitFor(() => expect(backend.sent('ops_extract')).toHaveLength(1));
    expect(backend.sent('ops_extract')[0]).toMatchObject({ dir: '', names: ['docs', 'readme.txt'] });
  });

  it('shows a refusal of the backend as it is, the missing unrar too, and the dialog stays', async () => {
    const message = 'Install unrar to open RAR archives.';
    await world({
      ops_extract: () => {
        throw new IpcFailure({ code: 'unsupported', message });
      },
    });
    await cursorTo(2);
    await key(listbox('Left'), 'F5');

    await key(screen.getByRole('textbox', { name: 'Target folder' }), 'Enter');

    expect(await screen.findByRole('alert')).toHaveTextContent(message);
    expect(screen.getByRole('dialog', { name: 'Extract “readme.txt” to' })).toBeInTheDocument();
  });

  it('F5 on `..` with nothing selected extracts nothing', async () => {
    const backend = await world();

    await key(listbox('Left'), 'F5');

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(backend.sent('ops_extract')).toEqual([]);
  });

  it('Alt+F9 in an archive is F5, and the Files menu offers it with its key', async () => {
    await world();
    await cursorTo(3);

    await key(listbox('Left'), 'F9', { altKey: true });
    expect(screen.getByRole('dialog', { name: 'Extract “notes.txt” to' })).toBeInTheDocument();
    await key(window, 'Escape');

    await fireEvent.click(screen.getByRole('button', { name: 'Files' }));
    expect(await screen.findByRole('menuitem', { name: 'Extract…' })).toHaveAttribute('aria-keyshortcuts', 'Alt+F9');
  });

  it('Alt+F9 in a folder extracts each selected archive whole, one job each, and leaves other files alone', async () => {
    const backend = await world({ ops_extract: () => 3 }, false);
    await key(listbox('Left'), 'Insert'); // a.zip
    await key(listbox('Left'), 'Insert'); // b.txt
    await key(listbox('Left'), 'Insert'); // f.7z

    await key(listbox('Left'), 'F9', { altKey: true });
    expect(screen.getByRole('dialog', { name: 'Extract 2 items to' })).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Target folder' })).toHaveValue('/right');
    await key(screen.getByRole('textbox', { name: 'Target folder' }), 'Enter');

    await vi.waitFor(() => expect(backend.sent('ops_extract')).toHaveLength(2));
    expect(backend.sent('ops_extract')).toEqual([
      { archive: '/left/a.zip', dir: '', names: [], to: '/right' },
      { archive: '/left/f.7z', dir: '', names: [], to: '/right' },
    ]);
  });

  it('Alt+F9 on a file that is no archive says to choose one', async () => {
    const backend = await world({}, false);
    await key(listbox('Left'), 'ArrowDown'); // b.txt

    await key(listbox('Left'), 'F9', { altKey: true });

    expect(await screen.findByText('Choose an archive to extract.')).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(backend.sent('ops_extract')).toEqual([]);
  });
});

describe('the password of an archive', () => {
  it('is asked for when extracting needs it; a wrong one is asked again, the right one runs the extraction again', async () => {
    let tries = 0;
    const backend = await world({
      ops_extract: () => {
        tries += 1;
        return tries === 1 ? needsPassword() : 9;
      },
      archive_password: (args: { password: string }) => args.password === 'right',
    });
    await cursorTo(2);
    await key(listbox('Left'), 'F5');
    await key(screen.getByRole('textbox', { name: 'Target folder' }), 'Enter');

    expect(await screen.findByRole('dialog', { name: 'Password for “a.zip”' })).toBeInTheDocument();
    expect(screen.getByText('This archive needs a password.')).toBeInTheDocument();
    const field = screen.getByLabelText('Password');
    expect(field).toHaveAttribute('type', 'password');
    expect(field).toHaveAttribute('autocomplete', 'off');
    expect(field).toHaveFocus();

    await fireEvent.input(field, { target: { value: 'wrong' } });
    await key(field, 'Enter');
    expect(await screen.findByText('The password is wrong. Try again.')).toBeInTheDocument();
    expect(field).toHaveValue('');
    expect(backend.sent('ops_extract')).toHaveLength(1);

    await fireEvent.input(field, { target: { value: 'right' } });
    await key(field, 'Enter');

    await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(backend.sent('archive_password')).toEqual([
      { archive: '/left/a.zip', password: 'wrong' },
      { archive: '/left/a.zip', password: 'right' },
    ]);
    expect(backend.sent('ops_extract')).toHaveLength(2);
  });

  it('giving up fails the extraction and starts nothing', async () => {
    const backend = await world({ ops_extract: () => needsPassword() });
    await cursorTo(2);
    await key(listbox('Left'), 'F5');
    await key(screen.getByRole('textbox', { name: 'Target folder' }), 'Enter');
    await screen.findByRole('dialog', { name: 'Password for “a.zip”' });

    await key(window, 'Escape');

    expect(await screen.findByText('The archive needs a password.')).toBeInTheDocument();
    expect(screen.queryByRole('dialog', { name: 'Password for “a.zip”' })).not.toBeInTheDocument();
    expect(backend.sent('ops_extract')).toHaveLength(1);
    expect(backend.sent('archive_password')).toEqual([]);
  });

  it('is asked for when opening the archive in the pane, which then lists it', async () => {
    let tries = 0;
    const backend = await world(
      {
        archive_open: () => {
          tries += 1;
          return tries === 1 ? needsPassword() : null;
        },
        archive_password: () => true,
      },
      false,
    );
    await key(listbox('Left'), 'Enter');

    const field = await screen.findByLabelText('Password');
    await fireEvent.input(field, { target: { value: 'secret' } });
    await key(field, 'Enter');

    await vi.waitFor(() => expect(backend.sent('archive_open')).toHaveLength(2));
    expect(backend.sent('archive_password')).toEqual([{ archive: '/left/a.zip', password: 'secret' }]);
  });
});

describe('F3 in an archive', () => {
  it('views the entry through the archive and refuses a folder', async () => {
    const backend = await world(viewAnswers(new FakeFile(numbered(10))));

    await key(listbox('Left'), 'F3'); // `..`
    await cursorTo(1);
    await key(listbox('Left'), 'F3'); // docs
    expect(backend.sent('archive_view_open')).toEqual([]);

    await cursorTo(1);
    await key(listbox('Left'), 'F3');

    expect(await screen.findByRole('dialog', { name: 'readme.txt' })).toBeInTheDocument();
    expect(backend.sent('archive_view_open')).toEqual([{ archive: '/left/a.zip', dir: '', name: 'readme.txt' }]);
    expect(backend.sent('view_open')).toEqual([]);
  });

  it('shows what the backend refuses (an entry too large) in the viewer', async () => {
    const message = 'This entry is too large to view; extract it first (F5).';
    await world({
      archive_view_open: () => {
        throw new IpcFailure({ code: 'unsupported', message });
      },
    });
    await cursorTo(2);

    await key(listbox('Left'), 'F3');

    expect(await screen.findByText(message)).toBeInTheDocument();
  });

  it('the quick view has nothing to show for an entry of an archive', async () => {
    await world(viewAnswers(new FakeFile(numbered(3))));
    await cursorTo(2);

    await key(window, 'q', { ctrlKey: true });

    expect(await screen.findByRole('region', { name: 'Quick view' })).toHaveTextContent('Nothing to show');
  });
});
