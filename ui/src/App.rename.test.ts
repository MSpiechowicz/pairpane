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

describe('renaming in place', () => {
  const field = () => screen.getByRole('textbox', { name: 'New name' }) as HTMLInputElement;

  it('F2 opens a field in the row with the stem selected; Enter renames and the cursor follows the name', async () => {
    const backend = await shown();

    expect(await key(listbox('Left'), 'F2')).toBe(false);
    expect(field()).toHaveValue('a.pdf');
    expect(field()).toHaveFocus();
    expect([field().selectionStart, field().selectionEnd]).toEqual([0, 1]);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(field().closest('[role="option"]')).toHaveClass('cursor');

    await fireEvent.input(field(), { target: { value: 'z.pdf' } });
    await key(field(), 'Enter');

    await vi.waitFor(() => expect(screen.queryByRole('textbox', { name: 'New name' })).not.toBeInTheDocument());
    expect(backend.sent('ops_rename')).toEqual([{ path: '/left', from: 'a.pdf', to: 'z.pdf' }]);
    expect(listbox('Left')).toHaveFocus();

    backend.list('left', '/left', [entry('b.txt'), entry('sub', 'dir'), entry('z.pdf')]);
    await tick();
    expect(left().getByRole('option', { name: /^z/ })).toHaveClass('cursor');
  });

  it('a folder is selected whole', async () => {
    await shown();
    await key(listbox('Left'), 'End');

    await key(listbox('Left'), 'F2');

    expect(field()).toHaveValue('sub');
    expect([field().selectionStart, field().selectionEnd]).toEqual([0, 3]);
  });

  it('Escape closes it without a call and gives the focus back to the list', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'F2');
    await fireEvent.input(field(), { target: { value: 'z.pdf' } });

    await key(field(), 'Escape');

    await vi.waitFor(() => expect(screen.queryByRole('textbox', { name: 'New name' })).not.toBeInTheDocument());
    expect(backend.sent('ops_rename')).toEqual([]);
    await vi.waitFor(() => expect(listbox('Left')).toHaveFocus());
    expect(left().getByRole('option', { name: /^a/ })).toBeInTheDocument();
  });

  it('leaving the field cancels it', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'F2');

    await fireEvent.blur(field());

    expect(screen.queryByRole('textbox', { name: 'New name' })).not.toBeInTheDocument();
    expect(backend.sent('ops_rename')).toEqual([]);
  });

  it('the unchanged name just closes; an empty one does nothing', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'F2');
    await fireEvent.input(field(), { target: { value: '' } });
    await key(field(), 'Enter');
    expect(field()).toBeInTheDocument();

    await fireEvent.input(field(), { target: { value: 'a.pdf' } });
    await key(field(), 'Enter');

    expect(screen.queryByRole('textbox', { name: 'New name' })).not.toBeInTheDocument();
    expect(backend.sent('ops_rename')).toEqual([]);
  });

  it('a refused name keeps the field open and the reason is shown in the notice', async () => {
    await shown({ ops_rename: refuse('invalidInput', 'A file with that name already exists.') });
    await key(listbox('Left'), 'F2');
    await fireEvent.input(field(), { target: { value: 'b.txt' } });

    await key(field(), 'Enter');

    expect(await screen.findByRole('region', { name: 'Notice' })).toHaveTextContent('A file with that name already exists.');
    expect(field()).toHaveValue('b.txt');
    expect(field()).not.toHaveAttribute('readonly');
  });

  it('a folder that is listed instead takes the field away without a call', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'F2');

    backend.list('left', '/elsewhere', [entry('a.pdf')]);
    await tick();

    expect(screen.queryByRole('textbox', { name: 'New name' })).not.toBeInTheDocument();
    expect(backend.sent('ops_rename')).toEqual([]);
  });

  it('an entry that is gone takes the field away', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'F2');

    backend.list('left', '/left', [entry('b.txt'), entry('sub', 'dir')]);
    await tick();

    await vi.waitFor(() => expect(screen.queryByRole('textbox', { name: 'New name' })).not.toBeInTheDocument());
    expect(backend.sent('ops_rename')).toEqual([]);
  });

  it('the field follows its entry when the listing is re-sorted under it', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'F2');
    await fireEvent.input(field(), { target: { value: 'typed' } });

    backend.list('left', '/left', [entry('0.txt'), entry('1.txt'), entry('a.pdf'), entry('b.txt'), entry('sub', 'dir')]);
    await tick();
    await tick();

    expect(field()).toHaveValue('typed');
    expect(field().closest('[role="option"]')).toBe(left().getAllByRole('option')[3]);
  });

  it('keys typed in the field are its own: F8 does not trash, Tab does not switch panes', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'F2');

    await key(field(), 'F8');
    await key(field(), 'Delete');
    await key(field(), 'Tab');

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(backend.sent('ops_trash')).toEqual([]);
    expect(pane('Left pane')).toHaveAttribute('aria-current', 'true');
  });

  it('a click or a double click in the field does not move the cursor, open the folder or take the focus', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'End'); // sub, a folder
    await key(listbox('Left'), 'F2');
    const opens = backend.sent('pane_open').length;

    await fireEvent.click(field());
    await fireEvent.dblClick(field());

    expect(backend.sent('pane_open')).toHaveLength(opens);
    expect(field()).toBeInTheDocument();
    expect(field()).toHaveFocus();
  });

  it('F2 on the `..` row does nothing', async () => {
    await shown();
    await key(listbox('Left'), 'Home');

    await key(listbox('Left'), 'F2');

    expect(screen.queryByRole('textbox', { name: 'New name' })).not.toBeInTheDocument();
  });

  it('Shift+F6 still renames in a dialog', async () => {
    await shown();

    await key(listbox('Left'), 'F6', { shiftKey: true });

    expect(screen.getByRole('dialog', { name: 'Rename' })).toBeInTheDocument();
  });
});
