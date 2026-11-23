import { describe, expect, it } from 'vitest';

import { fakeBackend } from '../../../tests/panes';
import { Unsaved } from './unsaved';

function world() {
  const backend = fakeBackend();
  const sent = () => backend.invoke.mock.calls.filter(([name]) => name === 'editor_set_dirty').map(([, args]) => args);

  return { unsaved: new Unsaved(backend.ipc), sent };
}

describe('the changes the backend is told of', () => {
  it('is told once when there are changes and once when there are none', () => {
    const { unsaved, sent } = world();

    unsaved.report('editor', true);
    unsaved.report('editor', true);
    unsaved.report('editor', false);
    unsaved.report('editor', false);

    expect(sent()).toEqual([{ dirty: true }, { dirty: false }]);
  });

  it('hears of the editor and the diff as one: clean only when both are', () => {
    const { unsaved, sent } = world();

    unsaved.report('editor', true);
    unsaved.report('diff', true);
    unsaved.report('editor', false);
    expect(unsaved.dirty).toBe(true);
    unsaved.report('diff', false);

    expect(unsaved.dirty).toBe(false);
    expect(sent()).toEqual([{ dirty: true }, { dirty: false }]);
  });

  it('hears of running transfers as one more reason to ask: clean only when nothing is left', () => {
    const { unsaved, sent } = world();

    unsaved.report('transfers', true);
    unsaved.report('editor', true);
    unsaved.report('transfers', false);
    expect(unsaved.dirty).toBe(true);
    unsaved.report('editor', false);

    expect(sent()).toEqual([{ dirty: true }, { dirty: false }]);
  });

  it('says again that there are changes only while there are', () => {
    const { unsaved, sent } = world();

    unsaved.stillDirty();
    unsaved.report('diff', true);
    unsaved.stillDirty();

    expect(sent()).toEqual([{ dirty: true }, { dirty: true }]);
  });
});
