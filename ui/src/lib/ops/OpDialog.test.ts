import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import { IpcFailure } from '$lib/ipc/transport';

import { opsBackend } from '../../../tests/ops';
import OpDialog from './OpDialog.svelte';
import type { OpRequest } from './targets';

const copy = (names: string[], to = '/backup'): OpRequest => ({ kind: 'copy', from: '/work', names, to });

function open(request: OpRequest, answers: Parameters<typeof opsBackend>[0] = {}) {
  const backend = opsBackend(answers);
  const onClose = vi.fn();
  const onDone = vi.fn();
  const onNotice = vi.fn();
  render(OpDialog, { request, ipc: backend.ipc, onClose, onDone, onNotice });
  return { ...backend, onClose, onDone, onNotice, field: screen.getByRole('textbox') as HTMLInputElement };
}

describe('OpDialog copy and move', () => {
  it('names one item, or counts several', () => {
    open(copy(['a.pdf']));
    expect(screen.getByRole('dialog', { name: 'Copy “a.pdf” to' })).toBeInTheDocument();
  });

  it('says Move and counts the items for a move', () => {
    open({ kind: 'move', from: '/work', names: ['a', 'b', 'c'], to: '/x' });

    expect(screen.getByRole('dialog', { name: 'Move 3 items to' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Move' })).toBeInTheDocument();
  });

  it('has the target filled in and focused, and verification off', () => {
    const { field } = open(copy(['a.pdf'], '/backup'));

    expect(field).toHaveValue('/backup');
    expect(field).toHaveFocus();
    expect(screen.getByRole('checkbox', { name: 'Verify after copy' })).not.toBeChecked();
  });

  it('Enter starts the job with exactly what was asked, and tells the owner', async () => {
    const { field, invoke, onDone, onClose } = open(copy(['a.pdf', 'b.txt']));
    await fireEvent.input(field, { target: { value: '/elsewhere' } });
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Verify after copy' }));

    await fireEvent.keyDown(field, { key: 'Enter' });

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledOnce());
    expect(invoke).toHaveBeenCalledWith('ops_start', {
      kind: 'copy',
      from: '/work',
      names: ['a.pdf', 'b.txt'],
      to: '/elsewhere',
      verify: true,
    });
    expect(onClose).not.toHaveBeenCalled();
  });

  it('the primary button starts it too; Enter in the checkbox as well', async () => {
    const { invoke, onDone } = open(copy(['a.pdf']));

    await fireEvent.keyDown(screen.getByRole('checkbox'), { key: 'Enter' });
    await vi.waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    await fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    await vi.waitFor(() => expect(onDone).toHaveBeenCalledTimes(2));

    expect(invoke).toHaveBeenCalledTimes(2);
  });

  it('Escape and Cancel close it without starting anything', async () => {
    const { invoke, onClose } = open(copy(['a.pdf']));

    await fireEvent.keyDown(window, { key: 'Escape' });
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(onClose).toHaveBeenCalledTimes(2);
    expect(invoke).not.toHaveBeenCalled();
  });

  it('Escape and Cancel do nothing while the job is being started, and the owner still hears of it', async () => {
    let started: () => void = () => {};
    const slow = () => new Promise<void>((resolve) => (started = resolve));
    const { field, onClose, onDone } = open(copy(['a.pdf']), { ops_start: slow });

    await fireEvent.keyDown(field, { key: 'Enter' });
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeDisabled();
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onClose).not.toHaveBeenCalled();

    started();
    await vi.waitFor(() => expect(onDone).toHaveBeenCalledOnce());
    expect(onClose).not.toHaveBeenCalled();
  });

  it('an empty target is not started', async () => {
    const { field, invoke } = open(copy(['a.pdf'], ''));

    await fireEvent.keyDown(field, { key: 'Enter' });

    expect(invoke).not.toHaveBeenCalled();
  });

  it('a refusal is shown in the dialog, which stays open and can try again', async () => {
    const refuse = async () => {
      throw new IpcFailure({ code: 'busy', message: 'Too many jobs are running.' });
    };
    const { field, onDone, onClose } = open(copy(['a.pdf']), { ops_start: refuse });

    await fireEvent.keyDown(field, { key: 'Enter' });

    expect(await screen.findByRole('alert')).toHaveTextContent('Too many jobs are running.');
    expect(onDone).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Copy' })).toBeEnabled();
  });
});

describe('OpDialog rename', () => {
  const rename = (name = 'report.pdf', dir = false): OpRequest => ({ kind: 'rename', path: '/work', name, dir });

  it('shows only the name, with the stem selected and not the extension', () => {
    const { field } = open(rename());

    expect(screen.getByRole('dialog', { name: 'Rename' })).toBeInTheDocument();
    expect(field).toHaveValue('report.pdf');
    expect([field.selectionStart, field.selectionEnd]).toEqual([0, 'report'.length]);
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument();
  });

  it('selects the whole name of a folder', () => {
    const { field } = open(rename('v1.2', true));

    expect([field.selectionStart, field.selectionEnd]).toEqual([0, 4]);
  });

  it('Enter renames in place and reports the new name', async () => {
    const { field, invoke, onDone } = open(rename());
    await fireEvent.input(field, { target: { value: 'final.pdf' } });

    await fireEvent.keyDown(field, { key: 'Enter' });

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledWith('final.pdf'));
    expect(invoke).toHaveBeenCalledWith('ops_rename', { path: '/work', from: 'report.pdf', to: 'final.pdf' });
  });

  it('the unchanged name just closes, without asking the backend', async () => {
    const { field, invoke, onDone, onClose } = open(rename());

    await fireEvent.keyDown(field, { key: 'Enter' });

    expect(onDone).toHaveBeenCalledWith('report.pdf');
    expect(onClose).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('an existing name is refused with the backend`s message, and the dialog stays', async () => {
    const exists = async () => {
      throw new IpcFailure({ code: 'invalidInput', message: 'A file with that name already exists.' });
    };
    const { field, onDone } = open(rename(), { ops_rename: exists });
    await fireEvent.input(field, { target: { value: 'taken.pdf' } });

    await fireEvent.keyDown(field, { key: 'Enter' });

    expect(await screen.findByRole('alert')).toHaveTextContent('A file with that name already exists.');
    expect(onDone).not.toHaveBeenCalled();
  });
});

describe('OpDialog new folder and new file', () => {
  const make = (kind: 'mkdir' | 'mkfile'): OpRequest => ({ kind, path: '/work' });

  it('a folder starts with an empty field, a hint about nested names, and makes the path as typed', async () => {
    const { field, invoke, onDone } = open(make('mkdir'), { ops_mkdir: () => 'a' });

    expect(screen.getByRole('dialog', { name: 'New folder' })).toBeInTheDocument();
    expect(screen.getByText(/Put \/ between names/)).toBeInTheDocument();
    expect(field).toHaveValue('');
    expect(field).toHaveAccessibleName('Folder name');
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument();
    await fireEvent.input(field, { target: { value: 'a/b' } });

    await fireEvent.click(screen.getByRole('button', { name: 'Create' }));

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledWith('a')); // the first folder, as the backend says
    expect(invoke).toHaveBeenCalledWith('ops_mkdir', { path: '/work', name: 'a/b' });
  });

  it('a file reports its name; there is no hint', async () => {
    const { field, invoke, onDone, onNotice } = open(make('mkfile'), { ops_mkfile: () => true });
    expect(screen.queryByText(/Put \//)).not.toBeInTheDocument();
    await fireEvent.input(field, { target: { value: 'notes.txt' } });

    await fireEvent.keyDown(field, { key: 'Enter' });

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledWith('notes.txt'));
    expect(invoke).toHaveBeenCalledWith('ops_mkfile', { path: '/work', name: 'notes.txt' });
    expect(onNotice).not.toHaveBeenCalled();
  });

  it('a file that was not opened is a notice before it is done', async () => {
    const { field, onDone, onNotice } = open(make('mkfile'), { ops_mkfile: () => false });
    await fireEvent.input(field, { target: { value: 'run.sh' } });

    await fireEvent.keyDown(field, { key: 'Enter' });

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledWith('run.sh'));
    expect(onNotice).toHaveBeenCalledWith({
      text: 'Created; not opened because it is a program or script',
      error: false,
    });
  });

  it('a file that was made but could not be opened is done, with the reason as an error notice', async () => {
    const failed = async () => {
      throw new IpcFailure({ code: 'process', message: 'Could not open it.' });
    };
    const { field, onDone, onNotice } = open(make('mkfile'), { ops_mkfile: failed });
    await fireEvent.input(field, { target: { value: 'notes.txt' } });

    await fireEvent.keyDown(field, { key: 'Enter' });

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledWith('notes.txt'));
    expect(onNotice).toHaveBeenCalledWith({ text: 'Could not open it.', error: true });
  });
});

describe('OpDialog links', () => {
  const link = (kind: 'symlink' | 'hardlink', name = 'report.pdf', dir = false): OpRequest => ({
    kind,
    path: '/work',
    name,
    dir,
    to: '/other',
  });

  it('names the target and the kind of link, starts with the target`s name and keeps its extension out of the selection', () => {
    const { field } = open(link('symlink'));

    expect(screen.getByRole('dialog', { name: 'Create symbolic link to “report.pdf”' })).toBeInTheDocument();
    expect(field).toHaveAccessibleName('Link name');
    expect(field).toHaveValue('report.pdf');
    expect([field.selectionStart, field.selectionEnd]).toEqual([0, 'report'.length]);
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument();
  });

  it('asks for a symbolic link with the arguments of the request, and tells the owner the link name', async () => {
    const { field, invoke, onDone } = open(link('symlink'));
    await fireEvent.input(field, { target: { value: 'shortcut.pdf' } });

    await fireEvent.keyDown(field, { key: 'Enter' });

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledWith('shortcut.pdf'));
    expect(invoke).toHaveBeenCalledWith('ops_link', {
      kind: 'symbolic',
      path: '/work',
      name: 'report.pdf',
      to: '/other',
      linkName: 'shortcut.pdf',
    });
  });

  it('asks for a hard link; the same name is fine (it goes to another folder)', async () => {
    const { field, invoke, onDone } = open(link('hardlink'));
    expect(screen.getByRole('dialog', { name: 'Create hard link to “report.pdf”' })).toBeInTheDocument();

    await fireEvent.keyDown(field, { key: 'Enter' });

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledWith('report.pdf'));
    expect(invoke).toHaveBeenCalledWith('ops_link', expect.objectContaining({ kind: 'hard', linkName: 'report.pdf' }));
  });

  it('selects the whole name of a folder', () => {
    const { field } = open(link('symlink', 'v1.2', true));

    expect([field.selectionStart, field.selectionEnd]).toEqual([0, 4]);
  });

  it('a refusal is shown, and the dialog stays', async () => {
    const refused = async () => {
      throw new IpcFailure({ code: 'io', message: 'Windows allows symbolic links only with Developer Mode on.' });
    };
    const { field, onDone } = open(link('symlink'), { ops_link: refused });

    await fireEvent.keyDown(field, { key: 'Enter' });

    expect(await screen.findByRole('alert')).toHaveTextContent('Windows allows symbolic links only');
    expect(onDone).not.toHaveBeenCalled();
  });
});

describe('OpDialog combine', () => {
  const combine: OpRequest = { kind: 'combine', from: '/work', name: 'movie.mkv.001', to: '/restore' };

  it('asks for the folder the parts are joined into, and starts the combine there', async () => {
    const { field, calls, onDone } = open(combine);
    expect(screen.getByRole('dialog', { name: 'Combine “movie.mkv.001” to' })).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Target folder' })).toHaveFocus();
    expect(field).toHaveValue('/restore');
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument();

    await fireEvent.input(field, { target: { value: '/elsewhere' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Combine' }));

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledOnce());
    expect(calls('ops_task')).toEqual([
      { from: '/work', names: ['movie.mkv.001'], task: { kind: 'combine', to: '/elsewhere' } },
    ]);
  });

  it('a refusal is shown, and the dialog stays', async () => {
    const { field, onDone } = open(combine, {
      ops_task: async () => {
        throw new IpcFailure({ code: 'invalidInput', message: 'choose exactly one item' });
      },
    });

    await fireEvent.keyDown(field, { key: 'Enter' });

    expect(await screen.findByRole('alert')).toHaveTextContent('choose exactly one item');
    expect(onDone).not.toHaveBeenCalled();
  });
});

describe('OpDialog add to an archive', () => {
  const add: OpRequest = { kind: 'add', archive: '/backup/a.zip', dir: 'docs/img', from: '/work', names: ['a.pdf', 'b.txt'] };

  it('shows the archive and its folder, which cannot be changed, and starts the add with the request', async () => {
    const { field, calls, onDone } = open(add);
    expect(screen.getByRole('dialog', { name: 'Copy 2 items to' })).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Archive folder' })).toHaveFocus();
    expect(field).toHaveValue('/backup/a.zip ▸ docs/img');
    expect(field).toHaveAttribute('readonly');
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument();

    await fireEvent.keyDown(field, { key: 'Enter' });

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledOnce());
    expect(calls('ops_add_to_archive')).toEqual([{ archive: '/backup/a.zip', dir: 'docs/img', from: '/work', names: ['a.pdf', 'b.txt'] }]);
    expect(calls('ops_start')).toEqual([]);
  });

  it('calls the root of the archive `/`', () => {
    const { field } = open({ ...add, dir: '' });

    expect(field).toHaveValue('/backup/a.zip ▸ /');
  });

  it('shows what the backend refuses (a kind it cannot write, a truncated archive) and stays', async () => {
    const message = 'This archive can’t be changed.';
    const { onDone } = open(add, {
      ops_add_to_archive: async () => {
        throw new IpcFailure({ code: 'unsupported', message });
      },
    });

    await fireEvent.click(screen.getByRole('button', { name: 'Copy' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(message);
    expect(onDone).not.toHaveBeenCalled();
  });
});
