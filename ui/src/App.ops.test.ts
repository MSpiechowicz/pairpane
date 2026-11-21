import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import type { OpsEvent } from '$lib/ipc/bindings/OpsEvent';
import { IpcFailure } from '$lib/ipc/transport';

import App from './App.svelte';
import { type Answers, appShown, appWorld, key, listbox, pane } from '../tests/app';
import { conflict, job } from '../tests/ops';
import { entry } from '../tests/panes';

const world = (answers: Answers = {}) => appWorld(invoke, answers);
const shown = (answers: Answers = {}) => appShown(invoke, answers);

describe('copy, move and rename from the keyboard', () => {
  beforeEach(() => {
    invoke.mockReset();
    window.localStorage.clear();
  });

  it('F5 is never left to the webview, which would reload the page: not even in a text field', async () => {
    await shown();

    expect(await key(screen.getByRole('textbox', { name: 'Shell command' }), 'F5')).toBe(false);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(await key(document.body, 'F5', { ctrlKey: true })).toBe(false);
  });

  it('F5 asks where to copy the item under the cursor, to the other pane`s folder; Enter starts it', async () => {
    const backend = await shown({ ops_start: () => 7 });

    expect(await key(listbox('Left'), 'F5')).toBe(false);
    const field = screen.getByRole('textbox', { name: 'Target folder' });
    expect(screen.getByRole('dialog', { name: 'Copy “a.pdf” to' })).toBeInTheDocument();
    expect(field).toHaveValue('/right');
    expect(field).toHaveFocus();

    await key(field, 'Enter');

    await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(backend.sent('ops_start')).toEqual([
      { kind: 'copy', from: '/left', names: ['a.pdf'], to: '/right', verify: false },
    ]);
    expect(listbox('Left')).toHaveFocus();
  });

  it('works on the active pane: the other way round when the right pane is active', async () => {
    const backend = await shown();
    await key(document.body, 'Tab');
    await key(listbox('Right'), 'ArrowDown');

    await key(listbox('Right'), 'F6');

    expect(screen.getByRole('dialog', { name: 'Move “c.md” to' })).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Target folder' })).toHaveValue('/left');
    expect(backend.sent('ops_start')).toEqual([]);
  });

  it('acts on the selection, and starting the job clears it', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'Insert');
    await key(listbox('Left'), 'Insert');
    expect(within(pane('Left pane')).getByText(/^Selected:/)).toBeInTheDocument();

    await key(listbox('Left'), 'F6');
    expect(screen.getByRole('dialog', { name: 'Move 2 items to' })).toBeInTheDocument();
    await key(screen.getByRole('textbox', { name: 'Target folder' }), 'Enter');

    await vi.waitFor(() => expect(backend.sent('ops_start')).toHaveLength(1));
    expect(backend.sent('ops_start')[0]).toMatchObject({ kind: 'move', names: ['a.pdf', 'b.txt'] });
    await vi.waitFor(() => expect(within(pane('Left pane')).queryByText(/^Selected:/)).not.toBeInTheDocument());
  });

  it('acts only on the selected items the quick filter shows, and the footer counts the same ones', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'a', { ctrlKey: true });
    await key(listbox('Left'), 't'); // only b.txt matches

    expect(within(pane('Left pane')).getByText(/^Selected: 0 folders · 1 file/)).toBeInTheDocument();
    await key(listbox('Left'), 'F6');
    expect(screen.getByRole('dialog', { name: 'Move “b.txt” to' })).toBeInTheDocument();
    await key(screen.getByRole('textbox', { name: 'Target folder' }), 'Enter');

    await vi.waitFor(() => expect(backend.sent('ops_start')).toHaveLength(1));
    expect(backend.sent('ops_start')[0]).toMatchObject({ names: ['b.txt'] });
  });

  it('Escape while the job is being started changes nothing: it starts, and the selection is cleared', async () => {
    let started: (value: null) => void = () => {};
    const backend = await shown({ ops_start: () => new Promise((resolve) => (started = resolve)) });
    await key(listbox('Left'), 'Insert');

    await key(listbox('Left'), 'F5');
    const field = screen.getByRole('textbox', { name: 'Target folder' });
    await key(field, 'Enter');
    await vi.waitFor(() => expect(backend.sent('ops_start')).toHaveLength(1));
    await key(field, 'Escape');
    expect(screen.getByRole('dialog')).toBeInTheDocument();

    started(null);
    await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(within(pane('Left pane')).queryByText(/^Selected:/)).not.toBeInTheDocument();
  });

  it('Escape closes the dialog and nothing is started, the selection stays', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'Insert');

    await key(listbox('Left'), 'F5');
    await key(screen.getByRole('textbox', { name: 'Target folder' }), 'Escape');

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(backend.sent('ops_start')).toEqual([]);
    expect(within(pane('Left pane')).getByText(/^Selected:/)).toBeInTheDocument();
  });

  it('the `..` row is no item: nothing opens', async () => {
    await shown();
    await key(listbox('Left'), 'Home');

    await key(listbox('Left'), 'F5');

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('the dock`s F5 and F6 buttons open the same dialogs', async () => {
    await shown();

    await fireEvent.click(screen.getByRole('button', { name: 'Copy (F5)' }));
    expect(screen.getByRole('dialog', { name: 'Copy “a.pdf” to' })).toBeInTheDocument();
    await key(screen.getByRole('textbox', { name: 'Target folder' }), 'Escape');

    await fireEvent.click(screen.getByRole('button', { name: 'Move (F6)' }));
    expect(screen.getByRole('dialog', { name: 'Move “a.pdf” to' })).toBeInTheDocument();
  });

  it('while a dialog is open the shortcuts do not run behind it', async () => {
    await shown();
    await key(listbox('Left'), 'F5');

    await key(screen.getByRole('button', { name: 'Cancel' }), 't', { ctrlKey: true });
    await key(screen.getByRole('button', { name: 'Cancel' }), 'F6');
    await key(screen.getByRole('button', { name: 'Cancel' }), 'Tab');

    expect(within(pane('Left pane')).queryAllByRole('button', { name: /^Close / })).toHaveLength(0);
    expect(screen.getAllByRole('dialog')).toHaveLength(1);
    expect(screen.getByRole('dialog', { name: /^Copy/ })).toBeInTheDocument();
    expect(pane('Left pane')).toHaveAttribute('aria-current', 'true');
  });

  it('Shift+F6 renames in place, and the cursor follows the new name once it is listed', async () => {
    const backend = await shown();

    await key(listbox('Left'), 'F6', { shiftKey: true });
    const field = screen.getByRole('textbox', { name: 'New name' }) as HTMLInputElement;
    expect(field).toHaveValue('a.pdf');
    expect([field.selectionStart, field.selectionEnd]).toEqual([0, 1]);

    await fireEvent.input(field, { target: { value: 'z.pdf' } });
    await key(field, 'Enter');

    await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(backend.sent('ops_rename')).toEqual([{ path: '/left', from: 'a.pdf', to: 'z.pdf' }]);

    backend.list('left', '/left', [entry('b.txt'), entry('sub', 'dir'), entry('z.pdf')]);
    await tick();
    const cursor = within(pane('Left pane')).getByRole('option', { name: /^z/ });
    expect(cursor).toHaveClass('cursor');
  });

  it('a rename that is refused says so in the dialog', async () => {
    await shown({
      ops_rename: async () => {
        throw new IpcFailure({ code: 'invalidInput', message: 'That name is taken.' });
      },
    });

    await key(listbox('Left'), 'F6', { shiftKey: true });
    const field = screen.getByRole('textbox', { name: 'New name' });
    await fireEvent.input(field, { target: { value: 'taken.txt' } });
    await key(field, 'Enter');

    expect(await screen.findByRole('alert')).toHaveTextContent('That name is taken.');
    expect(screen.getByRole('dialog', { name: 'Rename' })).toBeInTheDocument();
  });
});

describe('transfers in the app', () => {
  beforeEach(() => {
    invoke.mockReset();
    window.localStorage.clear();
  });

  it('there is no chip without jobs; a job shows it, and the panel does not open by itself', async () => {
    const backend = await shown();
    expect(screen.queryByRole('button', { name: /^Transfers/ })).not.toBeInTheDocument();

    backend.ops({ kind: 'added', job: job(1, { doneBytes: 1, totalBytes: 4 }) });

    expect(await screen.findByRole('button', { name: /^Transfers/ })).toHaveTextContent('⇅ 1 · 25 %');
    expect(screen.queryByRole('region', { name: 'Transfers' })).not.toBeInTheDocument();
  });

  it('the chip and Ctrl+J toggle the panel', async () => {
    const backend = await shown();
    backend.ops({ kind: 'added', job: job(1) });
    const chip = await screen.findByRole('button', { name: /^Transfers/ });

    await fireEvent.click(chip);
    expect(screen.getByRole('region', { name: 'Transfers' })).toBeInTheDocument();
    await fireEvent.click(chip);
    expect(screen.queryByRole('region', { name: 'Transfers' })).not.toBeInTheDocument();

    expect(await key(document.body, 'j', { ctrlKey: true })).toBe(false);
    expect(screen.getByRole('region', { name: 'Transfers' })).toBeInTheDocument();
    await key(screen.getByRole('button', { name: 'Pause a.pdf' }), 'j', { ctrlKey: true });
    expect(screen.queryByRole('region', { name: 'Transfers' })).not.toBeInTheDocument();
  });

  it('the update card and the panel are stacked in one place, so neither covers the other', async () => {
    const backend = await shown({ update_check: () => ({ version: '9.9.9', currentVersion: '0.1.0', date: null, notes: null }) });
    backend.ops({ kind: 'added', job: job(1) });
    await key(document.body, 'j', { ctrlKey: true });

    const card = await screen.findByRole('region', { name: 'Updates' });

    expect(card.parentElement).toBe(screen.getByRole('region', { name: 'Transfers' }).parentElement);
  });

  it('a question asks in a dialog; a snapshot after a reload asks again; several wait their turn', async () => {
    const backend = await shown();
    backend.ops({
      kind: 'snapshot',
      jobs: [job(1), job(2)],
      questions: [conflict(1, 1), conflict(2, 1, { name: 'b.txt', suggested: 'b (2).txt' })],
    });

    expect(await screen.findByRole('dialog', { name: '“a.pdf” already exists' })).toBeInTheDocument();
    expect(screen.getAllByRole('dialog')).toHaveLength(1);

    await fireEvent.click(screen.getByRole('button', { name: 'Overwrite' }));

    expect(backend.sent('ops_answer')).toEqual([{ job: 1, question: 1, answer: 'overwrite', all: false }]);
    expect(await screen.findByRole('dialog', { name: '“b.txt” already exists' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Skip' })).toHaveFocus();
  });
});
