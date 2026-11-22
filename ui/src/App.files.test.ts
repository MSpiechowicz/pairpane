import { fireEvent, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import { IpcFailure } from '$lib/ipc/transport';

import { type Answers, appShown, key, listbox, pane } from '../tests/app';
import { entry } from '../tests/panes';

const shown = (answers: Answers = {}) => appShown(invoke, answers);
const refuse = (code: 'invalidInput' | 'process' | 'notFound', message: string) => async () => {
  throw new IpcFailure({ code, message });
};
const left = () => within(pane('Left pane'));

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
});

describe('new folder', () => {
  it('F7 makes the folders of a path and the cursor goes to the first one once it is listed', async () => {
    const backend = await shown({ ops_mkdir: () => 'a' });

    await key(listbox('Left'), 'F7');
    const field = screen.getByRole('textbox', { name: 'Folder name' });
    expect(screen.getByRole('dialog', { name: 'New folder' })).toBeInTheDocument();
    expect(field).toHaveValue('');
    expect(field).toHaveFocus();

    await fireEvent.input(field, { target: { value: 'a/b/c' } });
    await key(field, 'Enter');

    await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(backend.sent('ops_mkdir')).toEqual([{ path: '/left', name: 'a/b/c' }]);

    backend.list('left', '/left', [entry('a', 'dir'), entry('a.pdf'), entry('b.txt'), entry('sub', 'dir')]);
    await tick();
    expect(left().getByRole('option', { name: /^a DIR/ })).toHaveClass('cursor');
  });

  it('a refusal is shown in the dialog, which stays open', async () => {
    await shown({ ops_mkdir: refuse('invalidInput', 'A folder with that name already exists.') });
    await key(listbox('Left'), 'F7');
    const field = screen.getByRole('textbox', { name: 'Folder name' });
    await fireEvent.input(field, { target: { value: 'sub' } });

    await key(field, 'Enter');

    expect(await screen.findByRole('alert')).toHaveTextContent('A folder with that name already exists.');
    expect(screen.getByRole('dialog', { name: 'New folder' })).toBeInTheDocument();
  });

  it('an empty name is not sent, and the `..` row is no obstacle', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'Home');

    await key(listbox('Left'), 'F7');
    await key(screen.getByRole('textbox', { name: 'Folder name' }), 'Enter');

    expect(backend.sent('ops_mkdir')).toEqual([]);
  });

  it('the dock`s F7 button opens the same dialog', async () => {
    await shown();

    await fireEvent.click(screen.getByRole('button', { name: 'New folder (F7)' }));

    expect(screen.getByRole('dialog', { name: 'New folder' })).toBeInTheDocument();
  });
});

describe('new file', () => {
  it('Shift+F4 makes an empty file and the cursor goes to it', async () => {
    const backend = await shown({ ops_mkfile: () => true });

    await key(listbox('Left'), 'F4', { shiftKey: true });
    const field = screen.getByRole('textbox', { name: 'File name' });
    expect(screen.getByRole('dialog', { name: 'New file' })).toBeInTheDocument();
    await fireEvent.input(field, { target: { value: 'notes.txt' } });
    await key(field, 'Enter');

    await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(backend.sent('ops_mkfile')).toEqual([{ path: '/left', name: 'notes.txt' }]);
    expect(screen.queryByRole('region', { name: 'Notice' })).not.toBeInTheDocument();

    backend.list('left', '/left', [entry('a.pdf'), entry('b.txt'), entry('notes.txt'), entry('sub', 'dir')]);
    await tick();
    expect(left().getByRole('option', { name: /^notes/ })).toHaveClass('cursor');
  });

  it('a program or script is made but not opened, and the notice says so', async () => {
    await shown({ ops_mkfile: () => false });
    await key(listbox('Left'), 'F4', { shiftKey: true });
    const field = screen.getByRole('textbox', { name: 'File name' });
    await fireEvent.input(field, { target: { value: 'run.sh' } });

    await key(field, 'Enter');

    expect(await screen.findByRole('region', { name: 'Notice' })).toHaveTextContent(
      'Created; not opened because it is a program or script',
    );
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('a file that could not be opened is kept: the cursor goes to it and the notice has the reason', async () => {
    const backend = await shown({ ops_mkfile: refuse('process', 'The file was made, but could not be opened.') });
    await key(listbox('Left'), 'F4', { shiftKey: true });
    const field = screen.getByRole('textbox', { name: 'File name' });
    await fireEvent.input(field, { target: { value: 'notes.txt' } });

    await key(field, 'Enter');

    expect(await screen.findByRole('region', { name: 'Notice' })).toHaveTextContent(
      'The file was made, but could not be opened.',
    );
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    backend.list('left', '/left', [entry('notes.txt'), entry('a.pdf')]);
    await tick();
    expect(left().getByRole('option', { name: /^notes/ })).toHaveClass('cursor');
  });

  it('any other refusal stays in the dialog', async () => {
    await shown({ ops_mkfile: refuse('invalidInput', 'A file with that name already exists.') });
    await key(listbox('Left'), 'F4', { shiftKey: true });
    const field = screen.getByRole('textbox', { name: 'File name' });
    await fireEvent.input(field, { target: { value: 'a.pdf' } });

    await key(field, 'Enter');

    expect(await screen.findByRole('alert')).toHaveTextContent('A file with that name already exists.');
  });

  it('Shift+F7 is nothing', async () => {
    await shown();

    await key(listbox('Left'), 'F7', { shiftKey: true });

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});

describe('the Files menu', () => {
  const choose = async (name: string | RegExp) => {
    await fireEvent.click(screen.getByRole('button', { name: 'Files' }));
    await fireEvent.click(screen.getByRole('menuitem', { name }));
  };

  it('is a real menu with the actions and their keys', async () => {
    await shown();
    const button = screen.getByRole('button', { name: 'Files' });
    expect(button).toHaveAttribute('aria-expanded', 'false');

    await fireEvent.click(button);

    expect(button).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getAllByRole('menuitem').map((item) => item.textContent?.replace(/\s+/g, ' ').trim())).toEqual([
      'New folder F7',
      'New file Shift+F4',
      'Edit in built-in editor',
      'Rename F2',
      'Multi-rename… Ctrl+M',
      'Delete F8',
      'Create symbolic link…',
      'Create hard link…',
      'Properties Alt+Enter',
      'Change attributes…',
      'Create checksum file…',
      'Verify checksum file',
      'Split file…',
      'Combine files…',
      'Find files… Alt+F7',
      'Compare folders',
      'Synchronize folders…',
      'Pack… Alt+F5',
      'Extract… Alt+F9',
    ]);
  });

  it('closes on Escape, on a shortcut, on a click outside, and when Settings opens', async () => {
    await shown();
    const files = screen.getByRole('button', { name: 'Files' });

    await fireEvent.click(files);
    await key(window, 'Escape');
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();

    await fireEvent.click(files);
    await fireEvent.click(document.body);
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();

    await fireEvent.click(files);
    await key(files, 'F7');
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    expect(screen.getByRole('dialog', { name: 'New folder' })).toBeInTheDocument();
    await key(screen.getByRole('textbox', { name: 'Folder name' }), 'Escape');

    await fireEvent.click(files);
    await fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    expect(screen.getAllByRole('menu')).toHaveLength(1);
    expect(files).toHaveAttribute('aria-expanded', 'false');
  });

  it('New folder, New file, Rename and Delete do what their keys do', async () => {
    await shown();

    await choose('New folder');
    expect(screen.getByRole('dialog', { name: 'New folder' })).toBeInTheDocument();
    await key(screen.getByRole('textbox', { name: 'Folder name' }), 'Escape');
    expect(listbox('Left')).toHaveFocus();

    await choose('New file');
    expect(screen.getByRole('dialog', { name: 'New file' })).toBeInTheDocument();
    await key(screen.getByRole('textbox', { name: 'File name' }), 'Escape');

    await choose('Delete');
    expect(screen.getByRole('dialog', { name: 'Move “a.pdf” to the Trash?' })).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    await choose('Rename');
    expect(screen.getByRole('textbox', { name: 'New name' })).toHaveFocus();
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('an item with nothing to act on does nothing, and the list keeps the focus', async () => {
    await shown();
    await key(listbox('Left'), 'Home');

    await choose('Delete');
    await choose('Rename');
    await choose('Create hard link…');

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    await vi.waitFor(() => expect(listbox('Left')).toHaveFocus());
  });

  it('Create symbolic link… names the target, makes the link in the other pane and puts its cursor on it', async () => {
    const backend = await shown();

    await choose('Create symbolic link…');
    const field = screen.getByRole('textbox', { name: 'Link name' }) as HTMLInputElement;
    expect(screen.getByRole('dialog', { name: 'Create symbolic link to “a.pdf”' })).toBeInTheDocument();
    expect(field).toHaveValue('a.pdf');
    expect([field.selectionStart, field.selectionEnd]).toEqual([0, 1]);
    await fireEvent.input(field, { target: { value: 'link.pdf' } });
    await key(field, 'Enter');

    await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(backend.sent('ops_link')).toEqual([
      { kind: 'symbolic', path: '/left', name: 'a.pdf', to: '/right', linkName: 'link.pdf' },
    ]);

    backend.list('right', '/right', [entry('c.md'), entry('link.pdf', 'file')]);
    await tick();
    expect(within(pane('Right pane')).getByRole('option', { name: /^link/ })).toHaveClass('cursor');
    expect(left().getByRole('option', { name: /^a/ })).toHaveClass('cursor');
  });

  it('Create hard link… asks for a hard link, and a refusal stays in the dialog', async () => {
    const backend = await shown({ ops_link: refuse('invalidInput', 'Hard links work only for files.') });

    await choose('Create hard link…');
    expect(screen.getByRole('dialog', { name: 'Create hard link to “a.pdf”' })).toBeInTheDocument();
    await key(screen.getByRole('textbox', { name: 'Link name' }), 'Enter');

    expect(await screen.findByRole('alert')).toHaveTextContent('Hard links work only for files.');
    expect(backend.sent('ops_link')).toEqual([
      { kind: 'hard', path: '/left', name: 'a.pdf', to: '/right', linkName: 'a.pdf' },
    ]);
  });

  it('links to the item under the active pane`s cursor, into the other pane', async () => {
    const backend = await shown();
    await key(document.body, 'Tab');
    await key(listbox('Right'), 'ArrowDown');

    await choose('Create symbolic link…');
    await key(screen.getByRole('textbox', { name: 'Link name' }), 'Enter');

    await vi.waitFor(() => expect(backend.sent('ops_link')).toHaveLength(1));
    expect(backend.sent('ops_link')[0]).toMatchObject({ path: '/right', name: 'c.md', to: '/left' });
  });
});
