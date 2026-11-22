import { fireEvent, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import { IpcFailure } from '$lib/ipc/transport';

import { type Answers, appShown, key, listbox, pane } from '../tests/app';
import { properties } from '../tests/ops';
import { entry } from '../tests/panes';

const shown = (answers: Answers = {}) => appShown(invoke, { ops_task: () => 1, ops_properties: () => properties(), ...answers });
const left = () => pane('Left pane');

const choose = async (name: string | RegExp) => {
  await fireEvent.click(screen.getByRole('button', { name: 'Files' }));
  await fireEvent.click(screen.getByRole('menuitem', { name }));
};

const notice = () => screen.findByRole('region', { name: 'Notice' });

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
});

describe('Properties', () => {
  it('Alt+Enter shows the properties of the item under the cursor, and Close gives the list the focus back', async () => {
    const backend = await shown();

    await key(listbox('Left'), 'Enter', { altKey: true });

    expect(await screen.findByRole('dialog', { name: 'Properties of “a.pdf”' })).toBeInTheDocument();
    expect(await screen.findByText('Permissions')).toBeInTheDocument();
    expect(backend.sent('ops_properties')).toEqual([{ path: '/left', name: 'a.pdf' }]);

    await fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    await vi.waitFor(() => expect(listbox('Left')).toHaveFocus());
  });

  it('the Files menu item does the same', async () => {
    const backend = await shown();

    await choose('Properties');

    expect(await screen.findByRole('dialog', { name: 'Properties of “a.pdf”' })).toBeInTheDocument();
    expect(backend.sent('ops_properties')).toHaveLength(1);
  });

  it('a folder is measured by the pane, so its size shows in the Size column as well', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'ArrowDown');
    await key(listbox('Left'), 'ArrowDown');

    await key(listbox('Left'), 'Enter', { altKey: true });

    expect(await screen.findByRole('dialog', { name: 'Properties of “sub”' })).toBeInTheDocument();
    await vi.waitFor(() => expect(backend.sent('pane_folder_size')).toMatchObject([{ pane: 'left', path: '/left', name: 'sub' }]));
  });

  it('is nothing on the `..` row', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'Home');

    await key(listbox('Left'), 'Enter', { altKey: true });

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(backend.sent('ops_properties')).toEqual([]);
  });

  it('a dialog that is open keeps the other shortcuts to itself', async () => {
    await shown();
    await key(listbox('Left'), 'Enter', { altKey: true });
    await screen.findByText('Permissions');

    await key(screen.getByRole('button', { name: 'Close' }), 'F7');

    expect(screen.queryByRole('dialog', { name: 'New folder' })).not.toBeInTheDocument();
  });
});

describe('Change attributes', () => {
  it('acts on the selected items and keeps them selected', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'Insert');
    await key(listbox('Left'), 'Insert');

    await choose('Change attributes…');
    expect(screen.getByRole('dialog', { name: 'Change attributes of 2 items' })).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Owner execute' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Apply' }));

    await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(backend.sent('ops_task')).toEqual([
      {
        from: '/left',
        names: ['a.pdf', 'b.txt'],
        task: {
          kind: 'attributes',
          change: {
            mode: { kind: 'adjust', add: 0o100, remove: 0 },
            readOnly: null,
            hidden: null,
            system: null,
            archive: null,
            modified: null,
            accessed: null,
            recursive: false,
          },
        },
      },
    ]);
    expect(within(left()).getAllByRole('option', { selected: true })).toHaveLength(2);
    await vi.waitFor(() => expect(listbox('Left')).toHaveFocus());
  });

  it('is nothing on the `..` row', async () => {
    await shown();
    await key(listbox('Left'), 'Home');

    await choose('Change attributes…');

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});

describe('Create checksum file', () => {
  it('makes the file in the pane`s folder, takes the selection and puts the cursor on the new file', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'Insert');

    await choose('Create checksum file…');
    const field = screen.getByRole('textbox', { name: 'Checksum file' });
    expect(field).toHaveValue('a.pdf.sha256');
    await key(field, 'Enter');

    await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(backend.sent('ops_task')).toEqual([
      { from: '/left', names: ['a.pdf'], task: { kind: 'checksum', algorithm: 'sha256', output: 'a.pdf.sha256' } },
    ]);
    expect(within(left()).queryAllByRole('option', { selected: true })).toHaveLength(0);

    backend.list('left', '/left', [entry('a.pdf'), entry('a.pdf.sha256'), entry('b.txt'), entry('sub', 'dir')]);
    await tick();
    expect(within(left()).getByRole('option', { name: /^a\.pdf sha256/ })).toHaveClass('cursor');
  });

  it('a refusal stays in the dialog', async () => {
    await shown({
      ops_task: async () => {
        throw new IpcFailure({ code: 'unsupported', message: 'the job could not start' });
      },
    });

    await choose('Create checksum file…');
    await key(screen.getByRole('textbox', { name: 'Checksum file' }), 'Enter');

    expect(await screen.findByRole('alert')).toHaveTextContent('the job could not start');
  });
});

describe('Verify checksum file', () => {
  it('starts the check of the file under the cursor at once, with no dialog', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'ArrowDown');

    await choose('Verify checksum file');

    await vi.waitFor(() => expect(backend.sent('ops_task')).toHaveLength(1));
    expect(backend.sent('ops_task')).toEqual([{ from: '/left', names: ['b.txt'], task: { kind: 'verifyChecksum' } }]);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    await vi.waitFor(() => expect(listbox('Left')).toHaveFocus());
  });

  it('a folder is no checksum file, and says so', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'End');

    await choose('Verify checksum file');

    expect(await notice()).toHaveTextContent('Choose a checksum file to verify.');
    expect(backend.sent('ops_task')).toEqual([]);
  });

  it('a refusal of the backend is told in the card', async () => {
    await shown({
      ops_task: async () => {
        throw new IpcFailure({ code: 'busy', message: 'Too many things are running.' });
      },
    });

    await choose('Verify checksum file');

    expect(await notice()).toHaveTextContent('Too many things are running.');
  });
});

describe('Split file', () => {
  it('offers the other pane`s folder, and starts the split of the file under the cursor', async () => {
    const backend = await shown();
    backend.list('left', '/left', [entry('a.pdf', 'file', { size: 250 * 1024 ** 2 }), entry('b.txt')]);
    await tick();

    await choose('Split file…');
    expect(screen.getByRole('dialog', { name: 'Split “a.pdf”' })).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Target folder' })).toHaveValue('/right');
    await fireEvent.click(screen.getByRole('button', { name: 'Split' }));

    await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(backend.sent('ops_task')).toEqual([
      { from: '/left', names: ['a.pdf'], task: { kind: 'split', to: '/right', partSize: 100 * 1024 ** 2 } },
    ]);
  });

  it('a folder cannot be split', async () => {
    await shown();
    await key(listbox('Left'), 'End');

    await choose('Split file…');

    expect(await notice()).toHaveTextContent('Choose a file to split.');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});

describe('Pack', () => {
  it('Alt+F5 packs the cursor item into the other pane`s folder, and the Files menu does the same', async () => {
    const backend = await shown();

    await key(listbox('Left'), 'F5', { altKey: true });
    expect(screen.getByRole('dialog', { name: 'Pack “a.pdf”' })).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Archive' })).toHaveValue('/right/a.pdf.zip');
    await fireEvent.click(screen.getByRole('button', { name: 'Pack' }));

    await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(backend.sent('ops_pack')).toEqual([
      {
        from: '/left',
        names: ['a.pdf'],
        archive: '/right/a.pdf.zip',
        format: 'zip',
        level: 'normal',
        password: null,
        encryptNames: false,
      },
    ]);

    await choose('Pack…');
    expect(screen.getByRole('dialog', { name: 'Pack “a.pdf”' })).toBeInTheDocument();
  });

  it('packs the selection, named after the folder', async () => {
    await shown();
    await key(listbox('Left'), 'a', { ctrlKey: true });

    await key(listbox('Left'), 'F5', { altKey: true });

    expect(screen.getByRole('dialog', { name: 'Pack 3 items' })).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Archive' })).toHaveValue('/right/left.zip');
  });
});

describe('Combine files', () => {
  const withParts = async () => {
    const backend = await shown();
    backend.list('left', '/left', [entry('movie.mkv.001'), entry('movie.mkv.002'), entry('notes.txt')]);
    await tick();
    await key(listbox('Left'), 'Home');
    await key(listbox('Left'), 'ArrowDown');
    return backend;
  };

  it('joins the parts that go with the first one into the other pane`s folder', async () => {
    const backend = await withParts();

    await choose('Combine files…');
    expect(screen.getByRole('dialog', { name: 'Combine “movie.mkv.001” to' })).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Target folder' })).toHaveValue('/right');
    await fireEvent.click(screen.getByRole('button', { name: 'Combine' }));

    await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(backend.sent('ops_task')).toEqual([
      { from: '/left', names: ['movie.mkv.001'], task: { kind: 'combine', to: '/right' } },
    ]);
  });

  it('works on a first part only, and says so for anything else', async () => {
    const backend = await withParts();
    await key(listbox('Left'), 'ArrowDown');

    await choose('Combine files…');

    expect(await notice()).toHaveTextContent('Choose the first part of a split file, named like “video.mkv.001”.');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(backend.sent('ops_task')).toEqual([]);
  });
});
