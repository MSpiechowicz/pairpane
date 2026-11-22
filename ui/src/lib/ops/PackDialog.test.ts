import { fireEvent, render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { IpcFailure } from '$lib/ipc/transport';

import { opsBackend } from '../../../tests/ops';
import PackDialog from './PackDialog.svelte';
import { PACK_DEFAULTS, rememberPack } from './pack';

beforeEach(() => rememberPack(PACK_DEFAULTS));

function open(names = ['photos'], answers: Parameters<typeof opsBackend>[0] = {}) {
  const backend = opsBackend(answers);
  const onClose = vi.fn();
  const onDone = vi.fn();
  const { unmount } = render(PackDialog, { ipc: backend.ipc, path: '/work', names, to: '/backup', onClose, onDone });
  return { ...backend, onClose, onDone, unmount };
}

const archive = () => screen.getByRole('textbox', { name: 'Archive' }) as HTMLInputElement;
const format = () => screen.getByRole('combobox', { name: 'Format' }) as HTMLSelectElement;
const level = () => screen.getByRole('combobox', { name: 'Level' }) as HTMLSelectElement;
const password = () => screen.getByLabelText('Password') as HTMLInputElement;
const confirmation = () => screen.getByLabelText('Confirm password') as HTMLInputElement;
const names = () => screen.getByRole('checkbox', { name: 'Encrypt file names' }) as HTMLInputElement;
const pack = () => fireEvent.click(screen.getByRole('button', { name: 'Pack' }));
const type = (input: HTMLElement, value: string) => fireEvent.input(input, { target: { value } });
const choose = (list: HTMLSelectElement, value: string) => fireEvent.change(list, { target: { value } });

describe('PackDialog', () => {
  it('names the items, starts with the archive in the other pane`s folder and focuses its path', () => {
    open(['photos']);

    expect(screen.getByRole('dialog', { name: 'Pack “photos”' })).toBeInTheDocument();
    expect(archive()).toHaveValue('/backup/photos.zip');
    expect(archive()).toHaveFocus();
    expect(format()).toHaveValue('zip');
    expect(level()).toHaveValue('normal');
  });

  it('names several items after the folder they are in', () => {
    open(['a', 'b', 'c']);

    expect(screen.getByRole('dialog', { name: 'Pack 3 items' })).toBeInTheDocument();
    expect(archive()).toHaveValue('/backup/work.zip');
  });

  it('asks for the archive typed, from the folder of the items, with a password that is not sent when empty', async () => {
    const { calls, onDone } = open(['a', 'b']);
    await type(archive(), '/elsewhere/mine.zip');
    await choose(level(), 'maximum');

    await pack();

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledOnce());
    expect(calls('ops_pack')).toEqual([
      {
        from: '/work',
        names: ['a', 'b'],
        archive: '/elsewhere/mine.zip',
        format: 'zip',
        level: 'maximum',
        password: null,
        encryptNames: false,
      },
    ]);
  });

  it('a format swaps the extension of the path, and a path with no known extension stays', async () => {
    open();

    await choose(format(), 'tarZst');
    expect(archive()).toHaveValue('/backup/photos.tar.zst');
    await choose(format(), 'sevenZ');
    expect(archive()).toHaveValue('/backup/photos.7z');

    await type(archive(), '/backup/mine');
    await choose(format(), 'zip');
    expect(archive()).toHaveValue('/backup/mine');
  });

  it('a tar.zst has no Store, and a Store chosen before falls back to Normal', async () => {
    open();
    await choose(level(), 'store');

    await choose(format(), 'tarZst');

    expect([...level().options].map((option) => option.textContent)).toEqual(['Fast', 'Normal', 'Maximum']);
    expect(level()).toHaveValue('normal');
  });

  it('a tar.zst has no password: the fields are off and what was typed is gone', async () => {
    const { calls } = open();
    await type(password(), 'secret');
    await type(confirmation(), 'secret');

    await choose(format(), 'tarZst');

    expect(password()).toBeDisabled();
    expect(confirmation()).toBeDisabled();
    expect(password()).toHaveValue('');
    expect(confirmation()).toHaveValue('');

    await pack();

    await vi.waitFor(() => expect(calls('ops_pack')).toHaveLength(1));
    expect(calls('ops_pack')[0]).toMatchObject({ format: 'tarZst', password: null, encryptNames: false });
  });

  it('a password that is not confirmed is refused without asking', async () => {
    const { invoke } = open();
    await type(password(), 'secret');
    await type(confirmation(), 'secrat');

    expect(screen.getByText('The passwords do not match.')).toBeInTheDocument();
    await pack();

    expect(screen.getByRole('alert')).toHaveTextContent('The passwords do not match.');
    expect(invoke).not.toHaveBeenCalled();
  });

  it('a password typed twice is sent, and is gone from the fields once it was', async () => {
    const { calls, onDone } = open();
    await type(password(), 'secret');
    await type(confirmation(), 'secret');
    expect(password()).toHaveAttribute('type', 'password');
    expect(confirmation()).toHaveAttribute('autocomplete', 'off');

    await pack();

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledOnce());
    expect(calls('ops_pack')[0]).toMatchObject({ password: 'secret', encryptNames: false });
    expect(password()).toHaveValue('');
    expect(confirmation()).toHaveValue('');
  });

  it('file names are encrypted by default in a 7z with a password, and only there', async () => {
    const { calls } = open();
    expect(screen.queryByRole('checkbox', { name: 'Encrypt file names' })).not.toBeInTheDocument();

    await choose(format(), 'sevenZ');
    expect(names()).toBeChecked();
    expect(names()).toBeDisabled();

    await type(password(), 'secret');
    await type(confirmation(), 'secret');
    expect(names()).toBeEnabled();
    await pack();

    await vi.waitFor(() => expect(calls('ops_pack')).toHaveLength(1));
    expect(calls('ops_pack')[0]).toMatchObject({ format: 'sevenZ', password: 'secret', encryptNames: true });
  });

  it('a 7z with the names left readable is asked for so', async () => {
    const { calls } = open();
    await choose(format(), 'sevenZ');
    await type(password(), 'secret');
    await type(confirmation(), 'secret');

    await fireEvent.click(names());
    await pack();

    await vi.waitFor(() => expect(calls('ops_pack')).toHaveLength(1));
    expect(calls('ops_pack')[0]).toMatchObject({ password: 'secret', encryptNames: false });
  });

  it('keeps the format, level and names choice for the next dialog, never the password', async () => {
    const { onDone, unmount } = open();
    await choose(format(), 'sevenZ');
    await choose(level(), 'fast');
    await type(password(), 'secret');
    await type(confirmation(), 'secret');
    await fireEvent.click(names());
    await pack();
    await vi.waitFor(() => expect(onDone).toHaveBeenCalledOnce());
    unmount();

    open();

    expect(format()).toHaveValue('sevenZ');
    expect(level()).toHaveValue('fast');
    expect(archive()).toHaveValue('/backup/photos.7z');
    expect(names()).not.toBeChecked();
    expect(password()).toHaveValue('');
  });

  it('an empty archive path asks for nothing', async () => {
    const { invoke } = open();
    await type(archive(), ' ');

    await pack();

    expect(invoke).not.toHaveBeenCalled();
  });

  it('a refusal is shown, and the dialog stays', async () => {
    const { onDone } = open(['photos'], {
      ops_pack: async () => {
        throw new IpcFailure({ code: 'invalidInput', message: 'The archive needs the extension .zip.' });
      },
    });

    await pack();

    expect(await screen.findByRole('alert')).toHaveTextContent('The archive needs the extension .zip.');
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
