import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import { IpcFailure } from '$lib/ipc/transport';

import { opsBackend } from '../../../tests/ops';
import { entry } from '../../../tests/panes';
import SplitDialog from './SplitDialog.svelte';

const MB = 1024 ** 2;

function open(size = 250 * MB, answers: Parameters<typeof opsBackend>[0] = {}) {
  const backend = opsBackend(answers);
  const onClose = vi.fn();
  const onDone = vi.fn();
  render(SplitDialog, {
    ipc: backend.ipc,
    path: '/work',
    entry: entry('movie.mkv', 'file', { size }),
    to: '/backup',
    onClose,
    onDone,
  });
  return { ...backend, onClose, onDone, target: screen.getByRole('textbox', { name: 'Target folder' }) as HTMLInputElement };
}

const sizeList = () => screen.getByRole('combobox', { name: 'Part size' }) as HTMLSelectElement;
const split = () => fireEvent.click(screen.getByRole('button', { name: 'Split' }));
const task = (calls: (name: 'ops_task') => unknown[]) => (calls('ops_task')[0] as { task: unknown }).task;

describe('SplitDialog', () => {
  it('names the file, has the other pane`s folder focused, and says how many parts', () => {
    const { target } = open();

    expect(screen.getByRole('dialog', { name: 'Split “movie.mkv”' })).toBeInTheDocument();
    expect(target).toHaveValue('/backup');
    expect(target).toHaveFocus();
    expect(screen.getByText('“movie.mkv” is 250 MB: 3 parts.')).toBeInTheDocument();
  });

  it('offers the floppy, Zip, CD and FAT32 sizes and a custom one', () => {
    open();

    expect([...sizeList().options].map((option) => option.textContent)).toEqual([
      'Floppy disk (1,457,664 bytes)',
      'Zip disk (100 MB)',
      'CD (700 MB)',
      'FAT32 (4 GB − 1 byte)',
      'Custom size',
    ]);
    expect(sizeList()).toHaveValue(String(100 * MB));
  });

  it('asks for the size that was chosen, from the folder of the file, into the folder typed', async () => {
    const { calls, onDone, target } = open();
    await fireEvent.input(target, { target: { value: '/elsewhere' } });
    await fireEvent.change(sizeList(), { target: { value: String(734_003_200) } });
    expect(screen.getByText('“movie.mkv” is 250 MB: 1 part.')).toBeInTheDocument();

    await split();

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledOnce());
    expect(calls('ops_task')).toEqual([
      { from: '/work', names: ['movie.mkv'], task: { kind: 'split', to: '/elsewhere', partSize: 734_003_200 } },
    ]);
  });

  it('takes a custom size in KB, MB or GB, as binary units', async () => {
    const { calls, target } = open();
    await fireEvent.change(sizeList(), { target: { value: 'custom' } });

    const amount = screen.getByRole('spinbutton', { name: 'Custom size' });
    await fireEvent.input(amount, { target: { value: '1.5' } });
    await fireEvent.change(screen.getByRole('combobox', { name: 'Unit' }), { target: { value: String(MB) } });
    expect(screen.getByText('“movie.mkv” is 250 MB: 167 parts.')).toBeInTheDocument();
    await fireEvent.keyDown(target, { key: 'Enter' });

    await vi.waitFor(() => expect(calls('ops_task')).toHaveLength(1));
    expect(task(calls)).toEqual({ kind: 'split', to: '/backup', partSize: 1_572_864 });
  });

  it('refuses parts under 1 KB without asking', async () => {
    const { invoke } = open();
    await fireEvent.change(sizeList(), { target: { value: 'custom' } });
    await fireEvent.change(screen.getByRole('combobox', { name: 'Unit' }), { target: { value: '1024' } });
    await fireEvent.input(screen.getByRole('spinbutton', { name: 'Custom size' }), { target: { value: '0.5' } });

    await split();

    expect(screen.getByRole('alert')).toHaveTextContent('Parts must be at least 1 KB.');
    expect(invoke).not.toHaveBeenCalled();
  });

  it('refuses an empty file without asking', async () => {
    const { invoke } = open(0);

    await split();

    expect(screen.getByRole('alert')).toHaveTextContent('“movie.mkv” is empty: there is nothing to split.');
    expect(invoke).not.toHaveBeenCalled();
  });

  it('refuses a split into more than 999 parts without asking', async () => {
    const { invoke } = open(5 * 1024 ** 3);
    await fireEvent.change(sizeList(), { target: { value: String(1_457_664) } });

    expect(screen.getByText('That would make 3684 parts; the most is 999. Choose bigger parts.')).toBeInTheDocument();
    await split();

    expect(screen.getByRole('alert')).toHaveTextContent('the most is 999');
    expect(invoke).not.toHaveBeenCalled();
  });

  it('an empty target asks for nothing', async () => {
    const { invoke, target } = open();
    await fireEvent.input(target, { target: { value: '' } });

    await split();

    expect(invoke).not.toHaveBeenCalled();
  });

  it('a refusal is shown, and the dialog stays', async () => {
    const { onDone } = open(250 * MB, {
      ops_task: async () => {
        throw new IpcFailure({ code: 'notFound', message: 'That folder does not exist.' });
      },
    });

    await split();

    expect(await screen.findByRole('alert')).toHaveTextContent('That folder does not exist.');
    expect(onDone).not.toHaveBeenCalled();
  });

  it('Escape and Cancel leave without asking', async () => {
    const { invoke, onClose } = open();

    await fireEvent.keyDown(window, { key: 'Escape' });
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(onClose).toHaveBeenCalledTimes(2);
    expect(invoke).not.toHaveBeenCalled();
  });
});
