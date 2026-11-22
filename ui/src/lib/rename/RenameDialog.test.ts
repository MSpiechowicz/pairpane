import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import type { RenamePreview } from '$lib/ipc/bindings/RenamePreview';
import type { RenameProblem } from '$lib/ipc/bindings/RenameProblem';
import type { RenameRow } from '$lib/ipc/bindings/RenameRow';
import type { RenameRule } from '$lib/ipc/bindings/RenameRule';
import { IpcFailure } from '$lib/ipc/transport';

import { opsBackend } from '../../../tests/ops';
import RenameDialog from './RenameDialog.svelte';
import { blankRule } from './rule';
import { SavedRenames } from './saved.svelte';

type Answers = Parameters<typeof opsBackend>[0];

/** A preview for `rows`: the names that change, and the problems there are. */
const rowsOf = (rows: RenameRow[]): RenamePreview => ({
  kind: 'rows',
  rows,
  changed: rows.filter((row) => row.from !== row.to).length,
  problems: rows.filter((row) => row.problem !== null).length,
});

const row = (from: string, to: string, problem: RenameProblem | null = null): RenameRow => ({ from, to, problem });

/** Stands in for the backend: every name gets the search text replaced by the replacement, nothing is wrong with any. */
const replacing = ({ names, rule }: { names: string[]; rule: RenameRule }): RenamePreview =>
  rowsOf(names.map((name) => row(name, rule.search === '' ? name : name.replace(rule.search, rule.replace))));

function open(answers: Answers = {}, names = ['a.txt', 'b.txt'], renames = new SavedRenames()) {
  const backend = opsBackend({ rename_preview: replacing as never, ops_task: () => 7, ...answers });
  const onClose = vi.fn();
  const onDone = vi.fn();
  render(RenameDialog, { ipc: backend.ipc, path: '/work', names, renames, onClose, onDone });
  return { ...backend, renames, onClose, onDone };
}

const field = (name: string) => screen.getByRole('textbox', { name }) as HTMLInputElement;
const type = (target: HTMLElement, value: string) => fireEvent.input(target, { target: { value } });
const apply = () => screen.getByRole('button', { name: 'Rename' }) as HTMLButtonElement;
const shown = () =>
  within(screen.getByRole('table', { name: 'Preview' }))
    .getAllByRole('row')
    .slice(1)
    .map((tableRow) => within(tableRow).getAllByRole('cell').map((cell) => cell.textContent));

/** Waits for the preview of the rule as it is now to arrive. */
const settled = () => vi.waitFor(() => expect(screen.getByRole('table', { name: 'Preview' })).toHaveAttribute('aria-busy', 'false'));

describe('RenameDialog preview', () => {
  it('names the items, focuses the name template and asks the backend about them with the rule it starts with', async () => {
    const { calls } = open();

    expect(screen.getByRole('dialog', { name: 'Rename 2 items' })).toBeInTheDocument();
    expect(field('Name')).toHaveFocus();
    await settled();

    expect(calls('rename_preview')).toEqual([{ path: '/work', names: ['a.txt', 'b.txt'], rule: blankRule() }]);
    expect(shown()).toEqual([
      ['a.txt', 'a.txt', ''],
      ['b.txt', 'b.txt', ''],
    ]);
    expect(screen.getByRole('status')).toHaveTextContent('2 items · 0 to rename');
  });

  it('shows the rows the backend sends and counts them, the unchanged ones dimmed', async () => {
    open({ rename_preview: () => rowsOf([row('a.txt', 'x.txt'), row('b.txt', 'b.txt')]) });
    await settled();

    const [changed, same] = within(screen.getByRole('table', { name: 'Preview' })).getAllByRole('row').slice(1);
    expect(changed).not.toHaveClass('unchanged');
    expect(same).toHaveClass('unchanged');
    expect(screen.getByRole('status')).toHaveTextContent('2 items · 1 to rename');
  });

  it('asks again once typing stops, with the rule the fields hold, and the table follows', async () => {
    const { calls } = open();
    await settled();

    await type(field('Search'), 'txt');
    await type(field('Replace with'), 'md');
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Match case' }));
    await vi.waitFor(() => expect(shown()[0][1]).toBe('a.md'));

    expect(calls('rename_preview')).toHaveLength(2); // one for the burst of typing, one at the start
    expect(calls('rename_preview').at(-1)).toMatchObject({ rule: { search: 'txt', replace: 'md', matchCase: true } });
    expect(shown()[0]).toEqual(['a.txt', 'a.md', '']);
  });

  it('drops an answer that arrives after a newer rule was asked about', async () => {
    const waiting: ((preview: RenamePreview) => void)[] = [];
    open({ rename_preview: () => new Promise((resolve) => waiting.push(resolve)) });
    await vi.waitFor(() => expect(waiting).toHaveLength(1));

    await type(field('Search'), 'a');
    await vi.waitFor(() => expect(waiting).toHaveLength(2));
    waiting[1](rowsOf([row('a.txt', 'new.txt')]));
    await vi.waitFor(() => expect(shown()).toEqual([['a.txt', 'new.txt', '']]));

    waiting[0](rowsOf([row('a.txt', 'stale.txt')]));
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(shown()).toEqual([['a.txt', 'new.txt', '']]);
    expect(apply()).toBeEnabled();
  });

  it('the names are shown with controls and bidirectional marks written out', async () => {
    open({ rename_preview: () => rowsOf([row('evil‮txt.exe', 'nice​.exe')]) }, ['evil‮txt.exe']);
    await settled();

    expect(shown()).toEqual([['evil⟨U+202E⟩txt.exe', 'nice⟨U+200B⟩.exe', '']]);
  });

  it('a preview the backend could not make is told, and nothing can be renamed', async () => {
    open({
      rename_preview: async () => {
        throw new IpcFailure({ code: 'invalidInput', message: 'Too many items.' });
      },
    });

    expect(await screen.findByRole('alert')).toHaveTextContent('Too many items.');
    expect(apply()).toBeDisabled();
  });
});

describe('RenameDialog rule fields', () => {
  it('a fault of the rule is told under its field and hides the rows', async () => {
    open({
      rename_preview: () => ({ kind: 'rule', error: { field: 'search', message: 'Unclosed group in “(a”.' } }),
    });

    expect(await screen.findByRole('alert')).toHaveTextContent('Unclosed group in “(a”.');
    expect(field('Search')).toHaveAttribute('aria-invalid', 'true');
    expect(field('Name')).not.toHaveAttribute('aria-invalid');
    expect(shown()).toEqual([]);
    expect(apply()).toBeDisabled();
  });

  it('a placeholder button writes at the caret of the template that had it last', async () => {
    open();
    await settled();
    const name = field('Name');
    name.focus();
    name.setSelectionRange(1, 2); // the N of [N] is selected: the token takes its place
    await fireEvent.click(screen.getByRole('button', { name: '[C]' }));

    await vi.waitFor(() => expect(name).toHaveValue('[[C]]'));
    expect(name).toHaveFocus();

    field('Extension').focus();
    await fireEvent.click(screen.getByRole('button', { name: '[YMD]' }));

    await vi.waitFor(() => expect(field('Extension')).toHaveValue('[E][YMD]'));
  });

  it('the counter fields send whole numbers only', async () => {
    const { calls } = open();
    await settled();
    const start = screen.getByRole('spinbutton', { name: 'Start' });

    await type(start, '7');
    await type(screen.getByRole('spinbutton', { name: 'Digits' }), '-3');
    await type(screen.getByRole('spinbutton', { name: 'Step' }), '2.5');
    await vi.waitFor(() => expect(calls('rename_preview')).toHaveLength(2));

    expect(calls('rename_preview').at(-1)).toMatchObject({ rule: { counter: { start: 7, step: 1, digits: 1 } } });
  });

  it('the case selects are sent, and the part is off while the case is kept', async () => {
    const { calls } = open();
    await settled();
    const part = screen.getByRole('combobox', { name: 'Applies to' });
    expect(part).toBeDisabled();

    await fireEvent.change(screen.getByRole('combobox', { name: 'Letters' }), { target: { value: 'title' } });
    await fireEvent.change(part, { target: { value: 'name' } });

    await vi.waitFor(() => expect(calls('rename_preview').at(-1)).toMatchObject({ rule: { case: 'title', casePart: 'name' } }));
    expect(part).toBeEnabled();
  });
});

describe('RenameDialog apply', () => {
  it('is off while the preview is awaited, with nothing to rename and while any row has a problem', async () => {
    let answer: RenamePreview = rowsOf([row('a.txt', 'a.txt'), row('b.txt', 'b.txt')]);
    open({ rename_preview: () => answer });
    expect(apply()).toBeDisabled(); // not asked yet

    await settled();
    expect(apply()).toBeDisabled(); // nothing changes

    answer = rowsOf([row('a.txt', 'x.txt'), row('b.txt', 'x.txt', 'duplicate')]);
    await type(field('Search'), 'x');
    await vi.waitFor(() => expect(screen.getByRole('status')).toHaveTextContent('1 problem'));

    expect(apply()).toBeDisabled();
    expect(shown()[1]).toEqual(['b.txt', 'x.txt', 'Another item gets the same name.']);
    expect(within(screen.getByRole('table', { name: 'Preview' })).getAllByRole('row')[2]).toHaveClass('bad');

    answer = rowsOf([row('a.txt', 'x.txt'), row('b.txt', 'y.txt')]);
    await type(field('Search'), 'xy');
    await vi.waitFor(() => expect(apply()).toBeEnabled());
  });

  it('sends the sources and the new names of the rows that change, in step, and not the ones that stay', async () => {
    const { calls, onDone } = open(
      { rename_preview: () => rowsOf([row('a.txt', 'one.txt'), row('b.txt', 'b.txt'), row('c.txt', 'three.txt')]) },
      ['a.txt', 'b.txt', 'c.txt'],
    );
    await vi.waitFor(() => expect(apply()).toBeEnabled());

    await fireEvent.click(apply());

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledOnce());
    expect(calls('ops_task')).toEqual([
      { from: '/work', names: ['a.txt', 'c.txt'], task: { kind: 'rename', targets: ['one.txt', 'three.txt'] } },
    ]);
  });

  it('Enter in a field does the same, and not while the apply is off', async () => {
    const { calls, onDone } = open({ rename_preview: () => rowsOf([row('a.txt', 'x.txt')]) }, ['a.txt']);
    await type(field('Replace with'), 'y'); // the answer is awaited: nothing yet
    await fireEvent.keyDown(field('Name'), { key: 'Enter' });
    expect(calls('ops_task')).toEqual([]);

    await vi.waitFor(() => expect(apply()).toBeEnabled());
    await fireEvent.keyDown(field('Name'), { key: 'Enter' });

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledOnce());
    expect(calls('ops_task')).toHaveLength(1);
  });

  it('a refusal is shown in the dialog, which stays', async () => {
    const { onDone, onClose } = open({
      rename_preview: () => rowsOf([row('a.txt', 'x.txt')]),
      ops_task: async () => {
        throw new IpcFailure({ code: 'invalidInput', message: 'The folder changed.' });
      },
    });
    await vi.waitFor(() => expect(apply()).toBeEnabled());

    await fireEvent.click(apply());

    expect(await screen.findByText('The folder changed.')).toBeInTheDocument();
    expect(onDone).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    expect(apply()).toBeEnabled();
  });

  it('Escape and Cancel close it without asking for anything', async () => {
    const { calls, onClose } = open();
    await settled();

    await fireEvent.keyDown(window, { key: 'Escape' });
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(onClose).toHaveBeenCalledTimes(2);
    expect(calls('ops_task')).toEqual([]);
  });
});

describe('RenameDialog presets', () => {
  it('Save as keeps the rule under a name; Load brings it back into the fields; Delete forgets it', async () => {
    const { renames } = open();
    await settled();
    await type(field('Search'), 'txt');
    await type(field('Replace with'), 'md');

    await fireEvent.click(screen.getByRole('button', { name: 'Save as…' }));
    await type(field('Name of the preset'), 'To markdown');
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    expect(renames.list).toEqual([{ name: 'To markdown', rule: expect.objectContaining({ search: 'txt', replace: 'md' }) }]);

    await type(field('Search'), '');
    await type(field('Replace with'), '');
    await fireEvent.change(screen.getByRole('combobox', { name: 'Saved preset' }), { target: { value: 'To markdown' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Load' }));

    expect(field('Search')).toHaveValue('txt');
    expect(field('Replace with')).toHaveValue('md');
    await vi.waitFor(() => expect(shown()[0][1]).toBe('a.md'));

    await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    expect(renames.list).toEqual([]);
  });

  it('a rule with a fault is not kept: the backend would drop it when it is loaded', async () => {
    const { renames } = open({ rename_preview: () => ({ kind: 'rule', error: { field: 'name', message: 'Unknown [Q].' } }) });
    await screen.findByRole('alert');

    await fireEvent.click(screen.getByRole('button', { name: 'Save as…' }));
    await type(field('Name of the preset'), 'Broken');
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    expect(renames.list).toEqual([]);
    expect(screen.getAllByRole('alert').map((alert) => alert.textContent)).toContain('Fix the rule before saving it.');
  });

  it('Enter in the name of a preset saves it and does not rename', async () => {
    const { renames, calls } = open({ rename_preview: () => rowsOf([row('a.txt', 'x.txt')]) }, ['a.txt']);
    await vi.waitFor(() => expect(apply()).toBeEnabled());
    await fireEvent.click(screen.getByRole('button', { name: 'Save as…' }));
    await type(field('Name of the preset'), 'Mine');

    await fireEvent.keyDown(field('Name of the preset'), { key: 'Enter' });

    expect(renames.list.map((preset) => preset.name)).toEqual(['Mine']);
    expect(calls('ops_task')).toEqual([]);
  });
});
