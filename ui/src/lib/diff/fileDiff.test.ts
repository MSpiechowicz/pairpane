import { describe, expect, it, vi } from 'vitest';

import type { EditOpened } from '$lib/ipc/bindings/EditOpened';
import type { EditSaved } from '$lib/ipc/bindings/EditSaved';

import { answer, PICKED, world } from '../../../tests/diff';
import { editDocument } from '../../../tests/editor';

describe('opening a comparison', () => {
  it('shows two text files as a text diff, each in a session of its own', async () => {
    const { diff, sent } = world();

    diff.open();
    await vi.waitFor(() => expect(diff.mode).toBe('text'));

    expect(diff.modal).toBe(true);
    expect(diff.pair?.left.text()).toBe('text of a.txt\n');
    expect(diff.pair?.right.text()).toBe('text of b.txt\n');
    expect(sent('edit_open')).toEqual([
      { path: '/one', name: 'a.txt', encoding: null },
      { path: '/two', name: 'b.txt', encoding: null },
    ]);
    expect(sent('view_open')).toHaveLength(0);
  });

  it.each<[string, EditOpened]>([
    ['binary', { kind: 'binary' }],
    ['larger than the editor takes', { kind: 'tooLarge', size: 9_000_000 }],
    ['not valid text', { kind: 'undecodable', encoding: 'UTF-8' }],
  ])('shows both files as bytes when one is %s', async (_, opened) => {
    const { diff, sent } = world({ 'b.txt': opened });

    diff.open();
    await vi.waitFor(() => expect(diff.mode).toBe('hex'));

    expect(diff.pair).toBeUndefined();
    expect(sent('view_open')).toEqual([
      { path: '/one', name: 'a.txt' },
      { path: '/two', name: 'b.txt' },
    ]);
    expect(diff.hex?.left.name).toBe('a.txt');
  });

  it('reads a file inside an archive read-only, as text or as bytes', async () => {
    const archived = { path: '/w/a.zip', name: 'in.txt', inside: 'docs' };
    const text = { kind: 'text', document: editDocument({ name: 'in.txt', text: 'archived\n', readOnly: true }) } satisfies EditOpened;
    const { diff, sent } = world({}, { archive_edit_open: (() => text) as never }, { pick: () => ({ left: PICKED.left, right: archived }) });

    diff.open();
    await vi.waitFor(() => expect(diff.mode).toBe('text'));

    expect(sent('archive_edit_open')).toEqual([{ archive: '/w/a.zip', dir: 'docs', name: 'in.txt', encoding: null }]);
    expect(diff.pair?.right.readOnly).toBe(true);
    expect(diff.pair?.right.text()).toBe('archived\n');
    expect(await diff.save('right')).toBe(false);
    expect(sent('edit_save')).toEqual([]);
    diff.close();

    const bytes = world({}, { archive_edit_open: (() => ({ kind: 'binary' })) as never, archive_view_open: (() => ({ session: 7, name: 'in.txt', size: 3, modified: null, kind: { kind: 'binary' }, encoding: '' })) as never }, { pick: () => ({ left: PICKED.left, right: archived }) });
    bytes.diff.open();
    await vi.waitFor(() => expect(bytes.diff.mode).toBe('hex'));
    expect(bytes.sent('archive_view_open')).toEqual([{ archive: '/w/a.zip', dir: 'docs', name: 'in.txt' }]);
  });

  it('says why nothing opens when the panes pick nothing, and opens nothing', () => {
    const { diff, say, pick, sent } = world();
    pick.mockReturnValue('Select two files, or put the cursor on a file in each pane.');

    diff.open();

    expect(say).toHaveBeenCalledWith('Select two files, or put the cursor on a file in each pane.', true);
    expect(diff.modal).toBe(false);
    expect(sent('edit_open')).toHaveLength(0);
  });

  it('says why a file did not open, and stays closed', async () => {
    const { diff, say } = world({}, { edit_open: (() => Promise.reject({ code: 'notFound', message: 'That file is gone.' })) as never });

    diff.open();

    await vi.waitFor(() => expect(say).toHaveBeenCalledWith('That file is gone.', true));
    expect(diff.modal).toBe(false);
  });

  it('opens one comparison at a time', async () => {
    const { diff, sent } = world();

    diff.open();
    diff.open();
    await vi.waitFor(() => expect(diff.mode).toBe('text'));
    diff.open();

    expect(sent('edit_open')).toHaveLength(2);
  });

  it('notes when the two files differ in line endings or in encoding, and says nothing when they match', async () => {
    const { diff } = world({
      'a.txt': { kind: 'text', document: editDocument({ name: 'a.txt', lineEnding: 'crlf', encoding: 'windows-1250' }) },
    });

    diff.open();
    await vi.waitFor(() => expect(diff.mode).toBe('text'));

    expect(diff.notes).toEqual([
      'Line endings differ (CRLF / LF); they are kept when saving.',
      'Encodings differ (windows-1250 / UTF-8); they are kept when saving.',
    ]);

    const same = world();
    same.diff.open();
    await vi.waitFor(() => expect(same.diff.mode).toBe('text'));
    expect(same.diff.notes).toEqual([]);
  });
});

describe('saving', () => {
  it('writes the side that was asked for, against that side`s revision, and the next save against the new one', async () => {
    const { diff, saves } = world();
    diff.open();
    await vi.waitFor(() => expect(diff.mode).toBe('text'));
    diff.pair?.right.setLineEnding('crlf');

    expect(await diff.save('right')).toBe(true);
    diff.pair?.right.setLineEnding('lf');
    expect(await diff.save('right')).toBe(true);

    expect(saves().map(({ name, options }) => [name, options.revision, options.lineEnding])).toEqual([
      ['b.txt', 'r-b.txt', 'crlf'],
      ['b.txt', 'next', 'lf'],
    ]);
    expect(diff.pair?.left.dirty).toBe(false);
  });

  it('joins a second save of the same side while one is under way', async () => {
    let release: (value: EditSaved) => void = () => {};
    const { diff, saves } = world({}, { edit_save: (() => new Promise<EditSaved>((resolve) => (release = resolve))) as never });
    diff.open();
    await vi.waitFor(() => expect(diff.mode).toBe('text'));

    const first = diff.save('left');
    const second = diff.save('left');
    release({ kind: 'saved', size: 1, revision: 'x' });

    expect(await Promise.all([first, second])).toEqual([true, true]);
    expect(saves()).toHaveLength(1);
  });

  it('asks before writing over a file that changed, and overwrites on the answer', async () => {
    const answers = [{ kind: 'changed', gone: false }, { kind: 'saved', size: 1, revision: 'forced' }];
    const { diff, saves } = world({}, { edit_save: (() => answers.shift()) as never });
    diff.open();
    await vi.waitFor(() => expect(diff.mode).toBe('text'));

    const saving = diff.save('left');
    await vi.waitFor(() => expect(diff.question?.title).toBe('The file changed on disk'));
    expect(diff.question).toMatchObject({ confirm: 'Overwrite', alternate: 'Reload', danger: true });
    answer(diff, 'confirm');

    expect(await saving).toBe(true);
    expect(saves().map(({ options }) => options.force)).toEqual([false, true]);
  });

  it('offers to save again a file that went, and reads again on Reload', async () => {
    const { diff } = world({}, { edit_save: (() => ({ kind: 'changed', gone: true })) as never });
    diff.open();
    await vi.waitFor(() => expect(diff.mode).toBe('text'));

    const gone = diff.save('right');
    await vi.waitFor(() => expect(diff.question?.title).toBe('The file was removed'));
    expect(diff.question).toMatchObject({ confirm: 'Save anyway', alternate: undefined });
    answer(diff, 'cancel');
    expect(await gone).toBe(false);

    const changed = world({}, { edit_save: (() => ({ kind: 'changed', gone: false })) as never });
    changed.diff.open();
    await vi.waitFor(() => expect(changed.diff.mode).toBe('text'));
    const saving = changed.diff.save('left');
    await vi.waitFor(() => expect(changed.diff.question).toBeDefined());
    answer(changed.diff, 'alternate');

    expect(await saving).toBe(false);
    expect(changed.sent('edit_open').filter((args) => args.name === 'a.txt')).toHaveLength(2);
    expect(changed.sent('edit_open').filter((args) => args.name === 'b.txt')).toHaveLength(1);
  });

  it('closes the comparison, with a word, when a file that is read again is no longer text', async () => {
    let opens = 0;
    const { diff, say } = world(
      {},
      {
        edit_open: ((args: { name: string }) => {
          opens += 1;
          return opens > 2 && args.name === 'a.txt'
            ? { kind: 'binary' }
            : { kind: 'text', document: editDocument({ name: args.name, revision: 'r' }) };
        }) as never,
        edit_save: (() => ({ kind: 'changed', gone: false })) as never,
      },
    );
    diff.open();
    await vi.waitFor(() => expect(diff.mode).toBe('text'));

    const saving = diff.save('left');
    await vi.waitFor(() => expect(diff.question).toBeDefined());
    answer(diff, 'alternate');
    await saving;

    expect(diff.modal).toBe(false);
    expect(say).toHaveBeenCalledWith('a.txt is no longer a text file.', true);
  });
});

describe('a side that is read again and is no longer text, while the other side has changes', () => {
  /** The left file turns binary when read again (its save is refused as changed, and the user answers Reload); the right file has changes. */
  async function leftTurnsBinary() {
    let opens = 0;
    const turned = world(
      {},
      {
        edit_open: ((args: { name: string }) => {
          opens += 1;
          return opens > 2 && args.name === 'a.txt'
            ? { kind: 'binary' }
            : { kind: 'text', document: editDocument({ name: args.name, revision: 'r' }) };
        }) as never,
        edit_save: ((args: { name: string }) =>
          args.name === 'a.txt' ? { kind: 'changed', gone: false } : { kind: 'saved', size: 1, revision: 'next' }) as never,
      },
    );
    turned.diff.open();
    await vi.waitFor(() => expect(turned.diff.mode).toBe('text'));
    turned.diff.pair?.right.setLineEnding('crlf');

    const saving = turned.diff.save('left');
    await vi.waitFor(() => expect(turned.diff.question?.title).toBe('The file changed on disk'));
    answer(turned.diff, 'alternate');
    await vi.waitFor(() => expect(turned.diff.question?.title).toBe('Save changes to b.txt?'));

    return { ...turned, saving };
  }

  it("asks about the other side's changes first, and Don't save closes without writing them", async () => {
    const { diff, say, saves, saving } = await leftTurnsBinary();
    expect(diff.modal).toBe(true);

    answer(diff, 'alternate');
    await saving;

    expect(diff.modal).toBe(false);
    expect(saves().map(({ name }) => name)).toEqual(['a.txt']);
    expect(say).toHaveBeenCalledWith('a.txt is no longer a text file.', true);
  });

  it('writes the other side on Save, then closes', async () => {
    const { diff, saves, saving } = await leftTurnsBinary();

    answer(diff, 'confirm');
    await saving;

    await vi.waitFor(() => expect(diff.modal).toBe(false));
    expect(saves().map(({ name }) => name)).toEqual(['a.txt', 'b.txt']);
  });

  it('stays open on Cancel with the other side kept, and the file that is no longer text locked', async () => {
    const { diff, say, saving } = await leftTurnsBinary();

    answer(diff, 'cancel');
    await saving;

    expect(diff.modal).toBe(true);
    expect(say).not.toHaveBeenCalled();
    expect(diff.pair?.right.dirty).toBe(true);
    expect(diff.pair?.left).toMatchObject({ unavailable: true, readOnly: true, failure: 'This file can no longer be compared as text.' });
    expect(await diff.save('left')).toBe(false);
  });
});

describe('unsaved changes', () => {
  async function dirtyBoth() {
    const world_ = world();
    world_.diff.open();
    await vi.waitFor(() => expect(world_.diff.mode).toBe('text'));
    world_.diff.pair?.left.setLineEnding('crlf');
    world_.diff.pair?.right.setLineEnding('crlf');

    return world_;
  }

  it('tells the backend once that there are changes, and once that there are none', async () => {
    const { diff, sent } = await dirtyBoth();
    expect(diff.dirty).toBe(true);
    expect(sent('editor_set_dirty')).toEqual([{ dirty: true }]);

    await diff.save('left');
    expect(sent('editor_set_dirty')).toEqual([{ dirty: true }]);
    await diff.save('right');

    expect(diff.dirty).toBe(false);
    expect(sent('editor_set_dirty')).toEqual([{ dirty: true }, { dirty: false }]);
  });

  it('closes at once when nothing is changed', async () => {
    const { diff, sent } = world();
    diff.open();
    await vi.waitFor(() => expect(diff.mode).toBe('text'));

    await diff.requestClose();

    expect(diff.modal).toBe(false);
    expect(diff.question).toBeUndefined();
    expect(sent('editor_set_dirty')).toEqual([]);
  });

  it('asks about both files, and Save writes both and closes', async () => {
    const { diff, saves, sent } = await dirtyBoth();

    const closing = diff.requestClose();
    await vi.waitFor(() => expect(diff.question).toBeDefined());
    expect(diff.question).toMatchObject({ title: 'Save changes to both files?', confirm: 'Save', alternate: "Don't save" });
    answer(diff, 'confirm');
    await closing;

    expect(saves().map(({ name }) => name)).toEqual(['a.txt', 'b.txt']);
    expect(diff.modal).toBe(false);
    expect(sent('editor_set_dirty')).toEqual([{ dirty: true }, { dirty: false }]);
  });

  it('names the one file that has changes', async () => {
    const { diff } = world();
    diff.open();
    await vi.waitFor(() => expect(diff.mode).toBe('text'));
    diff.pair?.right.setLineEnding('crlf');

    void diff.requestClose();
    await vi.waitFor(() => expect(diff.question?.title).toBe('Save changes to b.txt?'));
  });

  it('stays open when one of the two saves fails, with the other one saved', async () => {
    const saves = [{ kind: 'saved', size: 1, revision: 'x' }];
    const failing = world({}, { edit_save: (() => saves.shift() ?? Promise.reject({ code: 'io', message: 'The disk is full.' })) as never });
    failing.diff.open();
    await vi.waitFor(() => expect(failing.diff.mode).toBe('text'));
    failing.diff.pair?.left.setLineEnding('crlf');
    failing.diff.pair?.right.setLineEnding('crlf');

    const closing = failing.diff.requestClose();
    await vi.waitFor(() => expect(failing.diff.question).toBeDefined());
    answer(failing.diff, 'confirm');
    await closing;

    expect(failing.diff.modal).toBe(true);
    expect(failing.diff.pair?.left.dirty).toBe(false);
    expect(failing.diff.pair?.right.dirty).toBe(true);
    expect(failing.diff.pair?.right.failure).toBe('The disk is full.');
    expect(failing.diff.dirty).toBe(true);
  });

  it("Don't save closes and gives the changes up; Cancel keeps everything", async () => {
    const { diff, saves, sent } = await dirtyBoth();

    const cancelled = diff.requestClose();
    await vi.waitFor(() => expect(diff.question).toBeDefined());
    answer(diff, 'cancel');
    await cancelled;
    expect(diff.modal).toBe(true);

    const closing = diff.requestClose();
    await vi.waitFor(() => expect(diff.question).toBeDefined());
    answer(diff, 'alternate');
    await closing;

    expect(diff.modal).toBe(false);
    expect(saves()).toEqual([]);
    expect(sent('editor_set_dirty')).toEqual([{ dirty: true }, { dirty: false }]);
  });

  it('a window close with changes asks first: Don`t save lets it go, Cancel tells the backend again', async () => {
    const { diff, sent } = await dirtyBoth();

    const cancelled = diff.onCloseRequested();
    await vi.waitFor(() => expect(diff.question).toBeDefined());
    answer(diff, 'cancel');
    await cancelled;

    expect(sent('editor_close_ack')).toHaveLength(1);
    expect(sent('app_close_confirmed')).toHaveLength(0);
    expect(sent('editor_set_dirty')).toEqual([{ dirty: true }, { dirty: true }]);

    const closing = diff.onCloseRequested();
    await vi.waitFor(() => expect(diff.question).toBeDefined());
    answer(diff, 'alternate');
    await closing;

    expect(sent('app_close_confirmed')).toHaveLength(1);
  });

  it('a restart asks the same, and a later question replaces the earlier without a yes', async () => {
    const { diff } = await dirtyBoth();

    const first = diff.confirmDiscard();
    const second = diff.confirmDiscard();

    expect(await first).toBe(false);
    answer(diff, 'alternate');
    expect(await second).toBe(true);
  });

  it('a restart with nothing changed goes on at once', async () => {
    const { diff } = world();

    expect(await diff.confirmDiscard()).toBe(true);
  });
});

describe('a comparison of bytes', () => {
  it('has nothing to lose: it closes at once and ends both sessions on the backend', async () => {
    const { diff, sent } = world({ 'a.txt': { kind: 'binary' } });
    diff.open();
    await vi.waitFor(() => expect(diff.hex?.status).toBe('ready'));

    await diff.requestClose();

    expect(diff.modal).toBe(false);
    expect(sent('view_close')).toEqual([{ session: 1 }, { session: 2 }]);
    expect(diff.dirty).toBe(false);
  });

  it('a window close goes on at once', async () => {
    const { diff, sent } = world({ 'a.txt': { kind: 'binary' } });
    diff.open();
    await vi.waitFor(() => expect(diff.mode).toBe('hex'));

    await diff.onCloseRequested();

    expect(sent('app_close_confirmed')).toHaveLength(1);
  });
});
