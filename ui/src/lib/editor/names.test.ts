// A file name that reorders text (`invoice<RLO>fdp.exe`) is written out where the editor shows it: its title, its
// toolbar, the question about unsaved changes and the failures; the real name is what the backend is given.
import { fireEvent, screen, within } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

vi.mock('$lib/editor/cm', () => import('../../../tests/editor'));

import { startEditor } from '../../../tests/editor';

const NAME = 'invoice‮fdp.exe';
const SHOWN = 'invoice⟨U+202E⟩fdp.exe';
const target = { path: '/dir', name: NAME };

const type = (value: string) => fireEvent.input(screen.getByRole('textbox', { name: `Text of ${NAME}` }), { target: { value } });
const toolbarSave = () => within(screen.getByRole('toolbar', { name: 'Editor' })).getByRole('button', { name: 'Save' });

async function open(answers = {}) {
  const world = startEditor(answers, {}, target);
  await world.open();
  await screen.findByRole('textbox', { name: `Text of ${NAME}` });

  return world;
}

describe('the editor shows a file name with its characters that reorder text written out', () => {
  it('in its title and its toolbar, and asks the backend for the real name', async () => {
    const { sent } = await open();

    expect(screen.getByRole('dialog', { name: SHOWN })).toBeInTheDocument();
    expect(within(screen.getByRole('toolbar', { name: 'Editor' })).getByText(SHOWN)).toBeInTheDocument();
    expect(sent('edit_open')).toMatchObject([{ name: NAME }]);
  });

  it('in the question about unsaved changes', async () => {
    await open();
    await type('changed');

    await fireEvent.keyDown(window, { key: 'Escape' });

    expect(await screen.findByRole('dialog', { name: `Save changes to ${SHOWN}?` })).toBeInTheDocument();
  });

  it('in the question about a file that changed on disk', async () => {
    await open({ edit_save: () => ({ kind: 'changed', gone: false }) });
    await type('changed');

    await fireEvent.click(toolbarSave());

    const asked = await screen.findByRole('dialog', { name: 'The file changed on disk' });
    expect(within(asked).getByText(`${SHOWN} was changed by something else since it was opened.`)).toBeInTheDocument();
  });

  it('in the words that a file that is not text is refused with', async () => {
    const { open: opened } = startEditor({ edit_open: () => ({ kind: 'undecodable', encoding: 'UTF-8' }) }, {}, target);

    await opened();

    expect(await screen.findByText(`${SHOWN} is not valid UTF-8 text.`)).toBeInTheDocument();
  });

  it('in a failure the backend reports about it', async () => {
    await open({ edit_save: () => Promise.reject({ code: 'io', message: 'cannot write a‮txt.exe' }) });
    await type('changed');
    await fireEvent.keyDown(window, { key: 'Escape' });

    await fireEvent.click(within(await screen.findByRole('dialog', { name: /^Save changes/ })).getByRole('button', { name: 'Save' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('cannot write a⟨U+202E⟩txt.exe');
  });
});
