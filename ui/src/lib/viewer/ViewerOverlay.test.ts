import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import { FakeFile, numbered, openedSession } from '../../../tests/view';
import ViewerOverlay from './ViewerOverlay.svelte';

async function open(content = new FakeFile(numbered(300)), kind?: Parameters<typeof openedSession>[1], extra = {}) {
  const world = await openedSession(content, kind, extra);
  const onClose = vi.fn();
  render(ViewerOverlay, { session: world.session, onClose });

  return { ...world, onClose };
}

const box = () => screen.getByRole('textbox', { name: 'Search' });
const lines = () => Array.from(screen.getByRole('textbox', { name: /^Text of/ }).children).map((row) => row.textContent);
const marked = () => screen.getByText(/^line \d+$/, { selector: '[aria-current="true"]' });

async function find(text: string, init: KeyboardEventInit = {}) {
  await fireEvent.input(box(), { target: { value: text } });
  await fireEvent.keyDown(box(), { key: 'Enter', ...init });
}

describe('ViewerOverlay', () => {
  it('is a dialog named after the file, with the text in it focused', async () => {
    await open();

    expect(screen.getByRole('dialog', { name: 'file.txt' })).toBeInTheDocument();
    await vi.waitFor(() => expect(screen.getByRole('textbox', { name: /^Text of/ })).toHaveFocus());
    expect(await screen.findByText('line 0')).toBeInTheDocument();
  });

  it('Enter finds the next match, Shift+Enter the previous one, and the view goes to it', async () => {
    const { sent } = await open();
    await screen.findByText('line 0');

    await find('line 20');
    await vi.waitFor(() => expect(lines()[0]).toBe('line 20'));
    expect(marked()).toHaveTextContent('line 20');
    expect(sent('view_search')[0]).toMatchObject({
      query: { needle: { kind: 'text', text: 'line 20', caseSensitive: false }, from: 0, backward: false },
    });

    await new Promise((resolve) => setTimeout(resolve, 200));
    await fireEvent.keyDown(box(), { key: 'Enter' });
    await vi.waitFor(() => expect(lines()[0]).toBe('line 200'));

    await new Promise((resolve) => setTimeout(resolve, 200));
    await fireEvent.keyDown(box(), { key: 'Enter' });
    await vi.waitFor(() => expect(lines()[0]).toBe('line 201'));

    await new Promise((resolve) => setTimeout(resolve, 200));
    await fireEvent.keyDown(box(), { key: 'Enter', shiftKey: true });
    await vi.waitFor(() => expect(lines()[0]).toBe('line 200'));
    expect(sent('view_search').at(-1)).toMatchObject({ query: { backward: true } });
  });

  it('F3 and Shift+F3 search from anywhere in the viewer; the buttons do too', async () => {
    const { sent } = await open();
    await screen.findByText('line 0');
    await fireEvent.input(box(), { target: { value: 'line 7' } });

    await fireEvent.keyDown(window, { key: 'F3' });
    await vi.waitFor(() => expect(sent('view_search')).toHaveLength(1));
    await new Promise((resolve) => setTimeout(resolve, 200));
    await fireEvent.click(screen.getByRole('button', { name: 'Next match' }));
    await vi.waitFor(() => expect(sent('view_search')).toHaveLength(2));
    await new Promise((resolve) => setTimeout(resolve, 200));
    await fireEvent.click(screen.getByRole('button', { name: 'Previous match' }));
    await vi.waitFor(() => expect(sent('view_search')).toHaveLength(3));
    expect(sent('view_search')[2]).toMatchObject({ query: { backward: true } });
  });

  it('says when there is no match', async () => {
    await open();
    await screen.findByText('line 0');

    await find('nothing like this');

    expect(await screen.findByText('Not found')).toBeInTheDocument();
  });

  it('the case option is sent, and the hex option searches for bytes and refuses what is not hex', async () => {
    const { sent } = await open();
    await screen.findByText('line 0');

    await fireEvent.click(screen.getByRole('button', { name: 'Match case' }));
    await find('LINE 5');
    await vi.waitFor(() => expect(sent('view_search')).toHaveLength(1));
    expect(sent('view_search')[0]).toMatchObject({ query: { needle: { caseSensitive: true } } });
    expect(await screen.findByText('Not found')).toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: 'Search for hex bytes' }));
    expect(screen.getByRole('button', { name: 'Match case' })).toBeDisabled();
    await find('6c 69 6e 65 20 35');
    await vi.waitFor(() => expect(sent('view_search')).toHaveLength(2));
    expect(sent('view_search')[1]).toMatchObject({ query: { needle: { kind: 'hex', bytes: [108, 105, 110, 101, 32, 53] } } });

    await find('6c 6');
    expect(box()).toBeInvalid();
    expect(screen.getByText('Hex bytes, like 4d 5a')).toBeInTheDocument();
    expect(sent('view_search')).toHaveLength(2);
  });

  it('Escape stops a search that runs, and a second Escape closes the viewer', async () => {
    let answer: (hit: null) => void = () => {};
    const { sent, onClose } = await open(undefined, undefined, {
      view_search: () => new Promise((resolve) => (answer = resolve)),
    });
    await screen.findByText('line 0');
    await find('line 9');
    expect(await screen.findByText('Searching…')).toBeInTheDocument();

    await fireEvent.keyDown(box(), { key: 'Escape' });

    expect(sent('view_search_cancel')).toEqual([{ session: 1 }]);
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.queryByText('Searching…')).not.toBeInTheDocument();
    answer(null);
    await Promise.resolve();
    expect(screen.queryByText('Not found')).not.toBeInTheDocument();

    await fireEvent.keyDown(box(), { key: 'Escape' });
    expect(onClose).toHaveBeenCalledOnce();
  });

  it('the Cancel button stops the search too', async () => {
    const { sent } = await open(undefined, undefined, { view_search: () => new Promise(() => {}) });
    await screen.findByText('line 0');
    await find('line 9');

    await fireEvent.click(await screen.findByRole('button', { name: 'Cancel' }));

    expect(sent('view_search_cancel')).toHaveLength(1);
  });

  it('Text and Hex are modes of the same file; the hex view starts where the text was', async () => {
    const { session } = await open();
    await screen.findByText('line 0');
    await fireEvent.wheel(screen.getByRole('textbox', { name: /^Text of/ }), { deltaY: 18 * 5 });

    await fireEvent.click(screen.getByRole('button', { name: 'Hex' }));

    expect(await screen.findByRole('textbox', { name: /^Bytes of/ })).toBeInTheDocument();
    expect(session.mode).toBe('hex');
    await vi.waitFor(() => expect(screen.getByRole('textbox', { name: /^Bytes of/ }).children[0]).toHaveTextContent(/^00000020/));
  });

  it('the encoding select starts at the detected encoding and re-reads the text', async () => {
    const { sent } = await open();
    await screen.findByText('line 0');
    const select = screen.getByRole('combobox', { name: 'Encoding' });
    expect(select).toHaveValue('UTF-8');

    await fireEvent.change(select, { target: { value: 'windows-1250' } });

    await vi.waitFor(() => expect(sent('view_text')).toHaveLength(2));
    expect(sent('view_text')[1]).toMatchObject({ query: { encoding: 'windows-1250', snap: true } });
  });

  it('the Wrap button is a toggle of the text view', async () => {
    await open();
    await screen.findByText('line 0');
    const wrap = screen.getByRole('button', { name: 'Wrap' });
    expect(wrap).toHaveAttribute('aria-pressed', 'false');

    await fireEvent.click(wrap);

    expect(wrap).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('textbox', { name: /^Text of/ })).toHaveAttribute('data-wrap', 'true');
  });

  it('a kind with a preview of its own shows it, and Text and Hex are still there', async () => {
    const listing = { format: 'zip', entries: [{ name: 'a.txt', size: 5, dir: false, modified: null }], truncated: false };
    const { session } = await open(new FakeFile('PK\nhello\n'), () => ({ kind: 'archive', format: 'zip' }), {
      view_archive: () => listing,
    });

    expect(await screen.findByText('a.txt')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Preview' })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.queryByRole('textbox', { name: 'Search' })).not.toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: 'Text' }));
    expect(await screen.findByText('hello')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Hex' }));
    expect(await screen.findByRole('textbox', { name: /^Bytes of/ })).toBeInTheDocument();
    expect(session.mode).toBe('hex');
  });

  it('a binary file opens as bytes', async () => {
    await open(new FakeFile(Uint8Array.from([0, 1, 2, 3])), () => ({ kind: 'binary' }));

    expect(await screen.findByRole('textbox', { name: /^Bytes of/ })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Preview' })).not.toBeInTheDocument();
  });

  it('a file the backend refuses shows its message', async () => {
    const world = await openedSession(new FakeFile('x'));
    const failed = new (world.session.constructor as typeof import('./session.svelte').ViewSession)(
      {
        invoke: () => Promise.reject({ code: 'invalidInput', message: 'The viewer opens regular files, not links' }),
      } as never,
      '/dir',
      'link',
    );
    render(ViewerOverlay, { session: failed, onClose: vi.fn() });

    expect(await screen.findByRole('alert')).toHaveTextContent('The viewer opens regular files, not links');
    expect(screen.queryByRole('button', { name: 'Hex' })).not.toBeInTheDocument();
  });

  it('Close closes', async () => {
    const { onClose } = await open();

    await fireEvent.click(screen.getByRole('button', { name: 'Close' }));

    expect(onClose).toHaveBeenCalledOnce();
  });
});
