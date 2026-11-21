import { fireEvent, screen, within } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

vi.mock('$lib/editor/cm', () => import('../../../tests/editor'));

import { editDocument, openEditor, startEditor, type, typed } from '../../../tests/editor';

const key = (target: Element | Window, k: string, init: KeyboardEventInit = {}) =>
  fireEvent.keyDown(target, { key: k, ...init });

const asked = () => within(screen.getByRole('dialog', { name: /^Save changes/ }));
const conflict = async () => within(await screen.findByRole('dialog', { name: /^The file (changed|was removed)/ }));
const overlay = () => screen.queryByRole('dialog', { name: 'notes.txt' });
const dirty = () => screen.queryByRole('img', { name: 'Unsaved changes' });
const toolbarSave = () => within(screen.getByRole('toolbar', { name: 'Editor' })).getByRole('button', { name: 'Save' });
const reject = (code: string, message: string) => () => Promise.reject({ code, message });

describe('the editor overlay', () => {
  it('is a full dialog named after the file, with its text, the cursor position and the focus in the text', async () => {
    await openEditor();

    expect(overlay()).toBeInTheDocument();
    expect(typed()).toHaveValue('one\ntwo\n');
    await vi.waitFor(() => expect(typed()).toHaveFocus());
    expect(screen.getByLabelText('Cursor position')).toHaveTextContent('Ln 1, Col 1');
    expect(dirty()).not.toBeInTheDocument();
  });

  it('marks a changed text, and tells the backend so that closing the window can ask', async () => {
    const { sent } = await openEditor();

    await type('one\ntwo\nthree\n');

    expect(dirty()).toBeInTheDocument();
    expect(sent('editor_set_dirty')).toEqual([{ dirty: true }]);
  });

  it('closes at once when nothing is changed', async () => {
    const { sent } = await openEditor();

    await key(window, 'Escape');

    expect(overlay()).not.toBeInTheDocument();
    expect(sent('edit_save')).toHaveLength(0);
  });
});

describe('closing with changes', () => {
  it('asks Save, Don`t save or Cancel', async () => {
    await openEditor();
    await type('changed');

    await key(window, 'Escape');

    const dialog = asked();
    expect(dialog.getByRole('button', { name: 'Save' })).toBeInTheDocument();
    expect(dialog.getByRole('button', { name: "Don't save" })).toBeInTheDocument();
    expect(dialog.getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
    expect(overlay()).toBeInTheDocument();
  });

  it('Save writes the text with the file`s own choices and the revision it was read at, then closes', async () => {
    const { sent } = await openEditor({}, { encoding: 'windows-1250', lineEnding: 'crlf' });
    await type('changed');
    await key(window, 'Escape');

    await fireEvent.click(asked().getByRole('button', { name: 'Save' }));

    await vi.waitFor(() => expect(overlay()).not.toBeInTheDocument());
    expect(sent('edit_save')).toEqual([
      {
        path: '/dir',
        name: 'notes.txt',
        text: 'changed',
        options: { encoding: 'windows-1250', bom: false, lineEnding: 'crlf', revision: 'r1', force: false },
      },
    ]);
    expect(sent('editor_set_dirty')).toEqual([{ dirty: true }, { dirty: false }]);
  });

  it('Don`t save closes without writing anything', async () => {
    const { sent } = await openEditor();
    await type('changed');
    await key(window, 'Escape');

    await fireEvent.click(asked().getByRole('button', { name: "Don't save" }));

    await vi.waitFor(() => expect(overlay()).not.toBeInTheDocument());
    expect(sent('edit_save')).toHaveLength(0);
    expect(sent('editor_set_dirty')).toEqual([{ dirty: true }, { dirty: false }]);
  });

  it('Cancel, and Escape on the question, keep the editor and its changes', async () => {
    const { sent } = await openEditor();
    await type('changed');
    await key(window, 'Escape');

    await fireEvent.click(asked().getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('dialog', { name: /^Save changes/ })).not.toBeInTheDocument();
    await key(window, 'Escape');
    await key(window, 'Escape');

    expect(screen.queryByRole('dialog', { name: /^Save changes/ })).not.toBeInTheDocument();
    expect(overlay()).toBeInTheDocument();
    expect(typed()).toHaveValue('changed');
    expect(dirty()).toBeInTheDocument();
    expect(sent('edit_save')).toHaveLength(0);
  });

  it('the Close button asks the same', async () => {
    await openEditor();
    await type('changed');

    await fireEvent.click(screen.getByRole('button', { name: 'Close' }));

    expect(screen.getByRole('dialog', { name: /^Save changes/ })).toBeInTheDocument();
  });

  it('a save that is refused keeps the editor open', async () => {
    await openEditor({ edit_save: reject('io', 'the file cannot be written: no permission') });
    await type('changed');
    await key(window, 'Escape');

    await fireEvent.click(asked().getByRole('button', { name: 'Save' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('the file cannot be written: no permission');
    expect(overlay()).toBeInTheDocument();
    expect(dirty()).toBeInTheDocument();
  });
});

describe('a save that is under way', () => {
  /** A save that answers when `finish` is called. */
  function pending() {
    let finish: () => void = () => {};
    const answer = new Promise((resolve) => (finish = () => resolve({ kind: 'saved', size: 9, revision: 's1' })));

    return { answer: () => answer, finish };
  }

  it('does not keep the editor from closing: Don`t save still leaves, though the write never answers', async () => {
    const hung = pending();
    const { sent } = await openEditor({ edit_save: hung.answer });
    await type('changed');
    await fireEvent.click(toolbarSave());
    await vi.waitFor(() => expect(sent('edit_save')).toHaveLength(1));

    await key(window, 'Escape');
    await fireEvent.click(asked().getByRole('button', { name: "Don't save" }));

    await vi.waitFor(() => expect(overlay()).not.toBeInTheDocument());
  });

  it('is waited for by a Save on the way out, not written twice; the question can be asked again meanwhile', async () => {
    const slow = pending();
    const { sent } = await openEditor({ edit_save: slow.answer });
    await type('changed');
    await fireEvent.click(toolbarSave());
    await key(window, 'Escape');

    await fireEvent.click(asked().getByRole('button', { name: 'Save' }));
    await key(window, 'Escape');
    expect(asked().getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
    await fireEvent.click(asked().getByRole('button', { name: 'Cancel' }));
    slow.finish();

    await vi.waitFor(() => expect(dirty()).not.toBeInTheDocument());
    expect(sent('edit_save')).toHaveLength(1);
  });

  it('a Save on the way out writes again what was typed after the save began, and closes only once nothing is left', async () => {
    const slow = pending();
    const { sent } = await openEditor({ edit_save: slow.answer });
    await type('changed');
    await fireEvent.click(toolbarSave());
    await vi.waitFor(() => expect(sent('edit_save')).toHaveLength(1));
    await type('changed and more');
    await key(window, 'Escape');

    await fireEvent.click(asked().getByRole('button', { name: 'Save' }));
    expect(overlay()).toBeInTheDocument();
    slow.finish();

    await vi.waitFor(() => expect(overlay()).not.toBeInTheDocument());
    expect(sent('edit_save').map((args) => args.text)).toEqual(['changed', 'changed and more']);
  });

  it('a Save on the way out that does not get the later text written keeps the editor and the text', async () => {
    const slow = pending();
    let writes = 0;
    const { sent } = await openEditor({
      edit_save: () => (++writes === 1 ? slow.answer() : Promise.reject({ code: 'io', message: 'the disk is full' })),
    });
    await type('changed');
    await fireEvent.click(toolbarSave());
    await vi.waitFor(() => expect(sent('edit_save')).toHaveLength(1));
    await type('changed and more');
    await key(window, 'Escape');

    await fireEvent.click(asked().getByRole('button', { name: 'Save' }));
    slow.finish();

    expect(await screen.findByRole('alert')).toHaveTextContent('the disk is full');
    expect(overlay()).toBeInTheDocument();
    expect(typed()).toHaveValue('changed and more');
    expect(dirty()).toBeInTheDocument();
  });

  it('a Save on the way out that fails shows why and keeps the editor', async () => {
    const { sent } = await openEditor({ edit_save: reject('io', 'the disk is full') });
    await type('changed');
    await key(window, 'Escape');

    await fireEvent.click(asked().getByRole('button', { name: 'Save' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('the disk is full');
    expect(overlay()).toBeInTheDocument();
    expect(sent('edit_save')).toHaveLength(1);
  });
});

describe('a question replaced by another', () => {
  it('is ended without a yes, so nothing waits on it for ever; only the later one decides', async () => {
    const { editor } = await openEditor();
    await type('changed');

    const first = editor.confirmDiscard();
    const second = editor.confirmDiscard();

    expect(await first).toBe(false);
    await fireEvent.click(asked().getByRole('button', { name: "Don't save" }));
    expect(await second).toBe(true);
  });
});

describe('Tab in the overlay', () => {
  it('goes round through the toolbar`s selects and the text', async () => {
    await openEditor();
    const encoding = screen.getByRole('combobox', { name: 'Encoding' });

    expect(await key(encoding, 'Tab', { shiftKey: true })).toBe(false);
    expect(typed()).toHaveFocus();

    expect(await key(typed(), 'Tab')).toBe(false);
    expect(encoding).toHaveFocus();
  });
});

describe('saving', () => {
  it('Ctrl+S saves and stays; the changed mark goes and the backend hears of both', async () => {
    const { sent } = await openEditor();
    await type('changed');

    await key(typed(), 's', { ctrlKey: true });

    await vi.waitFor(() => expect(dirty()).not.toBeInTheDocument());
    expect(sent('edit_save')).toHaveLength(1);
    expect(overlay()).toBeInTheDocument();
    expect(sent('editor_set_dirty')).toEqual([{ dirty: true }, { dirty: false }]);
  });

  it('the next save is made against the revision the last one gave', async () => {
    const { sent, options } = await openEditor();
    await type('one');
    await fireEvent.click(toolbarSave());
    await vi.waitFor(() => expect(dirty()).not.toBeInTheDocument());

    await type('two');
    await fireEvent.click(toolbarSave());

    await vi.waitFor(() => expect(sent('edit_save')).toHaveLength(2));
    expect(options().map((option) => option.revision)).toEqual(['r1', 's1']);
  });

  it('a refusal is shown, and the text stays changed', async () => {
    await openEditor({ edit_save: reject('invalidInput', 'some characters cannot be saved in this encoding') });
    await type('changed');

    await fireEvent.click(toolbarSave());

    expect(await screen.findByRole('alert')).toHaveTextContent('some characters cannot be saved in this encoding');
    expect(dirty()).toBeInTheDocument();
    expect(typed()).toHaveValue('changed');
  });

  it('a file that cannot be written opens with a banner, and Save is off', async () => {
    const { sent } = await openEditor({}, { readOnly: true });

    expect(screen.getByText('This file is read-only: it cannot be changed or saved here.')).toBeInTheDocument();
    expect(toolbarSave()).toBeDisabled();

    await key(typed(), 's', { ctrlKey: true });
    expect(sent('edit_save')).toHaveLength(0);
  });

  it('says that a file with mixed line endings is saved with one', async () => {
    await openEditor({}, { mixedEndings: true, lineEnding: 'crlf' });

    expect(screen.getByText(/mixes line endings.*CRLF/)).toBeInTheDocument();
  });

  it('the line ending and the byte order mark are saved as chosen; a mark only goes with Unicode', async () => {
    const { sent } = await openEditor();
    expect(screen.getByRole('checkbox', { name: 'Byte order mark' })).not.toBeChecked();

    await fireEvent.change(screen.getByRole('combobox', { name: 'Line ending' }), { target: { value: 'cr' } });
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Byte order mark' }));
    expect(dirty()).toBeInTheDocument();
    await fireEvent.click(toolbarSave());

    await vi.waitFor(() => expect(sent('edit_save')).toHaveLength(1));
    expect(sent('edit_save')[0]).toMatchObject({ options: { bom: true, lineEnding: 'cr' } });

    await fireEvent.change(screen.getByRole('combobox', { name: 'Encoding' }), { target: { value: 'windows-1250' } });
    expect(screen.queryByRole('checkbox', { name: 'Byte order mark' })).not.toBeInTheDocument();
    await fireEvent.click(toolbarSave());

    await vi.waitFor(() => expect(sent('edit_save')).toHaveLength(2));
    expect(sent('edit_save')[1]).toMatchObject({ options: { bom: false, encoding: 'windows-1250' } });
  });
});

describe('a file that changed on disk', () => {
  /** Saves are refused as changed, until one is forced. */
  const changed = (gone: boolean) => (args: { options: { force: boolean } }) =>
    args.options.force ? { kind: 'saved', size: 9, revision: 'later' } : { kind: 'changed', gone };

  it('asks Overwrite, Reload or Cancel; Overwrite saves again, whatever changed', async () => {
    const { options } = await openEditor({ edit_save: changed(false) });
    await type('mine');
    await fireEvent.click(toolbarSave());

    await fireEvent.click((await conflict()).getByRole('button', { name: 'Overwrite' }));

    await vi.waitFor(() => expect(dirty()).not.toBeInTheDocument());
    expect(options().map((option) => option.force)).toEqual([false, true]);
  });

  it('Reload reads the file again and gives the changes up', async () => {
    const { sent } = await openEditor({ edit_save: changed(false) });
    await type('mine');
    await fireEvent.click(toolbarSave());

    await fireEvent.click((await conflict()).getByRole('button', { name: 'Reload' }));

    await vi.waitFor(() => expect(sent('edit_open')).toHaveLength(2));
    await vi.waitFor(() => expect(typed()).toHaveValue('one\ntwo\n'));
    expect(dirty()).not.toBeInTheDocument();
    expect(sent('edit_save')).toHaveLength(1);
  });

  it('Cancel, and Escape, leave everything as it is', async () => {
    const { options } = await openEditor({ edit_save: changed(false) });
    await type('mine');
    await fireEvent.click(toolbarSave());

    await fireEvent.click((await conflict()).getByRole('button', { name: 'Cancel' }));
    await fireEvent.click(toolbarSave());
    (await conflict()).getByRole('button', { name: 'Cancel' });
    await key(window, 'Escape');

    expect(screen.queryByRole('dialog', { name: /^The file/ })).not.toBeInTheDocument();
    expect(typed()).toHaveValue('mine');
    expect(dirty()).toBeInTheDocument();
    expect(options().map((option) => option.force)).toEqual([false, false]);
  });

  it('a file that is gone is offered Save anyway, and Cancel', async () => {
    const { sent } = await openEditor({ edit_save: changed(true) });
    await type('mine');
    await fireEvent.click(toolbarSave());

    const dialog = await conflict();
    expect(dialog.getByRole('button', { name: 'Save anyway' })).toBeInTheDocument();
    expect(dialog.queryByRole('button', { name: 'Reload' })).not.toBeInTheDocument();
    await fireEvent.click(dialog.getByRole('button', { name: 'Save anyway' }));

    await vi.waitFor(() => expect(dirty()).not.toBeInTheDocument());
    expect(sent('edit_save')[1]).toMatchObject({ options: { force: true } });
  });

  it('saving on the way out goes on to close once the file is overwritten', async () => {
    await openEditor({ edit_save: changed(false) });
    await type('mine');
    await key(window, 'Escape');
    await fireEvent.click(asked().getByRole('button', { name: 'Save' }));

    await fireEvent.click((await conflict()).getByRole('button', { name: 'Overwrite' }));

    await vi.waitFor(() => expect(overlay()).not.toBeInTheDocument());
  });
});

describe('the encoding', () => {
  it('on an unchanged text it reads the file again in the encoding chosen', async () => {
    const { sent } = await openEditor();

    await fireEvent.change(screen.getByRole('combobox', { name: 'Encoding' }), { target: { value: 'windows-1250' } });

    await vi.waitFor(() => expect(sent('edit_open')).toEqual([
      { path: '/dir', name: 'notes.txt', encoding: null },
      { path: '/dir', name: 'notes.txt', encoding: 'windows-1250' },
    ]));
    expect(screen.getByRole('combobox', { name: 'Encoding' })).toHaveValue('windows-1250');
  });

  it('on a changed text it only changes how the file is saved, and says so', async () => {
    const { sent } = await openEditor();
    await type('changed');

    await fireEvent.change(screen.getByRole('combobox', { name: 'Encoding' }), { target: { value: 'windows-1250' } });

    expect(sent('edit_open')).toHaveLength(1);
    expect(typed()).toHaveValue('changed');
    expect(screen.getByRole('status')).toHaveTextContent(/saved in/);
    await fireEvent.click(toolbarSave());
    await vi.waitFor(() => expect(sent('edit_save')).toHaveLength(1));
    expect(sent('edit_save')[0]).toMatchObject({ options: { encoding: 'windows-1250' } });
  });

  it('a file that does not read in it stays as it was, with the reason', async () => {
    let reads = 0;
    const { sent } = await openEditor({
      edit_open: (args: { encoding: string | null }) => {
        reads += 1;
        return reads === 1
          ? { kind: 'text', document: editDocument() }
          : { kind: 'undecodable', encoding: args.encoding };
      },
    });

    await fireEvent.change(screen.getByRole('combobox', { name: 'Encoding' }), { target: { value: 'UTF-16LE' } });

    expect(await screen.findByRole('alert')).toHaveTextContent('UTF-16LE');
    expect(typed()).toHaveValue('one\ntwo\n');
    expect(screen.getByRole('combobox', { name: 'Encoding' })).toHaveValue('UTF-8');
    expect(sent('edit_open')).toHaveLength(2);
  });
});

describe('a file whose bytes are not text in the encoding it was found in', () => {
  const unreadable = (args: { encoding: string | null }) =>
    args.encoding === null
      ? { kind: 'undecodable', encoding: 'UTF-8' }
      : { kind: 'text', document: editDocument({ text: 'zażółć', encoding: args.encoding }) };

  it('opens no text, and offers to read it as another encoding', async () => {
    const { open } = startEditor({ edit_open: unreadable });

    await open();

    expect(await screen.findByRole('alert')).toHaveTextContent('UTF-8');
    expect(screen.queryByRole('textbox', { name: /^Text of/ })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Reopen' })).toBeInTheDocument();
  });

  it('Reopen reads it in the encoding chosen, and the text opens', async () => {
    const { open, sent } = startEditor({ edit_open: unreadable });
    await open();
    await screen.findByRole('button', { name: 'Reopen' });

    await fireEvent.change(screen.getByRole('combobox', { name: 'Reopen as' }), { target: { value: 'windows-1250' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Reopen' }));

    expect(await screen.findByRole('textbox', { name: /^Text of/ })).toHaveValue('zażółć');
    expect(sent('edit_open')).toEqual([
      { path: '/dir', name: 'notes.txt', encoding: null },
      { path: '/dir', name: 'notes.txt', encoding: 'windows-1250' },
    ]);
  });

  it('Escape closes it; nothing was changed, so nothing is asked', async () => {
    const { open } = startEditor({ edit_open: unreadable });
    await open();
    await screen.findByRole('button', { name: 'Reopen' });

    await key(window, 'Escape');

    expect(overlay()).not.toBeInTheDocument();
  });
});
