import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import { FakeFile, numbered, openedSession } from '../../../tests/view';
import TextView from './TextView.svelte';

/** A thousand lines of `line N`; the view holds 40 rows (nothing is measured here) and asks for 120 lines. */
const file = () => new FakeFile(numbered(1000));
const FETCH = 120;
const ROWS = 40;

const shown = () => Array.from(screen.getByRole('textbox').children).map((row) => row.textContent);
const press = (key: string, init: KeyboardEventInit = {}) => fireEvent.keyDown(screen.getByRole('textbox'), { key, ...init });

async function open(content = file(), props: Record<string, unknown> = {}) {
  const world = await openedSession(content);
  const view = render(TextView, { session: world.session, ...props });
  await vi.waitFor(() => expect(world.sent('view_text')).not.toHaveLength(0));
  await vi.waitFor(() => expect(screen.getByRole('textbox')).toBeInTheDocument());
  if (content.size > 0) {
    await vi.waitFor(() => expect(shown().length).toBeGreaterThan(0));
  }

  return { ...world, view };
}

describe('TextView', () => {
  it('opens at the start with one window, and shows the rows that fit', async () => {
    const { sent, session } = await open();

    expect(sent('view_text')).toEqual([
      { session: 1, query: { encoding: 'UTF-8', offset: 0, backward: false, lines: FETCH, snap: true } },
    ]);
    expect(shown()).toHaveLength(ROWS + 1);
    expect(shown()[0]).toBe('line 0');
    expect(shown()[ROWS]).toBe('line 40');
    expect(session.position).toBe(0);
  });

  it('an empty file says so', async () => {
    await open(new FakeFile(''));

    expect(await screen.findByText('The file is empty.')).toBeInTheDocument();
  });

  it('the wheel and the arrow keys move by lines without a request per line', async () => {
    const { sent, session } = await open();

    await fireEvent.wheel(screen.getByRole('textbox'), { deltaY: 54 });
    expect(shown()[0]).toBe('line 3');
    await press('ArrowDown');
    await press('ArrowDown');
    expect(shown()[0]).toBe('line 5');
    await press('ArrowUp');
    expect(shown()[0]).toBe('line 4');

    expect(sent('view_text')).toHaveLength(1);
    expect(session.position).toBe(file().starts[4]);
  });

  it('PageDown and PageUp move by a page less a line', async () => {
    await open();

    await press('PageDown');
    expect(shown()[0]).toBe(`line ${ROWS - 1}`);
    await press('PageUp');
    expect(shown()[0]).toBe('line 0');
  });

  it('asks for the next window, from where the last one ended, as the view nears the end of its lines', async () => {
    const { sent } = await open();
    const content = file();

    await fireEvent.wheel(screen.getByRole('textbox'), { deltaY: 18 * 30 });
    expect(sent('view_text')).toHaveLength(1);
    await fireEvent.wheel(screen.getByRole('textbox'), { deltaY: 18 * 20 });

    await vi.waitFor(() => expect(sent('view_text')).toHaveLength(2));
    expect(sent('view_text')[1]).toEqual({
      session: 1,
      query: { encoding: 'UTF-8', offset: content.starts[FETCH], backward: false, lines: FETCH, snap: false },
    });
    expect(shown()[0]).toBe('line 50');
  });

  it('End asks for the lines before the end of the file, Home for the start', async () => {
    const { sent } = await open();
    const size = file().size;

    await press('End');
    await vi.waitFor(() => expect(shown().at(-1)).toBe('line 999'));
    expect(sent('view_text')[1]).toEqual({
      session: 1,
      query: { encoding: 'UTF-8', offset: size, backward: true, lines: FETCH, snap: false },
    });

    await press('Home');
    await vi.waitFor(() => expect(shown()[0]).toBe('line 0'));
    expect(sent('view_text')[2]).toMatchObject({ query: { offset: 0, backward: false, snap: false } });
  });

  it('scrolling up from the end asks for the window before the first line held, without snapping', async () => {
    const { sent } = await open();
    await press('End');
    await vi.waitFor(() => expect(shown().at(-1)).toBe('line 999'));
    const held = file().starts[1000 - FETCH];

    for (let step = 0; step < 50; step += 1) {
      await press('ArrowUp');
    }

    await vi.waitFor(() => expect(sent('view_text')).toHaveLength(3));
    expect(sent('view_text')[2]).toMatchObject({ query: { offset: held, backward: true, snap: false } });
    expect(shown()[0]).toBe('line 910');
  });

  it('dragging the scroll bar asks for the line at the offset it stands for, snapped; its end is the end of the file', async () => {
    const { sent, view } = await open();
    const track = view.container.querySelector<HTMLElement>('.track')!;

    track.scrollTop = 4000; // one pixel per byte here
    await fireEvent.scroll(track);
    await vi.waitFor(() => expect(sent('view_text')).toHaveLength(2));
    expect(sent('view_text')[1]).toMatchObject({ query: { offset: 4000, backward: false, snap: true } });
    await vi.waitFor(() => expect(shown()[0]).toBe('line 456'));

    track.scrollTop = file().size;
    await fireEvent.scroll(track);
    await vi.waitFor(() => expect(sent('view_text')).toHaveLength(3));
    expect(sent('view_text')[2]).toMatchObject({ query: { offset: file().size, backward: true, snap: false } });
    await vi.waitFor(() => expect(shown().at(-1)).toBe('line 999'));
  });

  it('a window that a newer jump has outdated is dropped when it arrives', async () => {
    const { session, view } = await open();
    const real = session.ipc.invoke.bind(session.ipc);
    let release = () => {};
    const gate = new Promise<void>((resolve) => (release = resolve));
    session.ipc.invoke = ((name: never, args: { query?: { offset: number } }) =>
      args.query?.offset === 100 ? gate.then(() => real(name, args as never)) : real(name, args as never)) as never;
    const track = view.container.querySelector<HTMLElement>('.track')!;

    track.scrollTop = 100;
    await fireEvent.scroll(track);
    track.scrollTop = 7000;
    await fireEvent.scroll(track);
    await vi.waitFor(() => expect(shown()[0]).toBe('line 790'));
    release();
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(shown()[0]).toBe('line 790');
  });

  it('scrolling while a jump is on its way does not mix a window of the old place into the new buffer', async () => {
    const { session, view, sent } = await open();
    const track = view.container.querySelector<HTMLElement>('.track')!;
    track.scrollTop = 4000;
    await fireEvent.scroll(track);
    await vi.waitFor(() => expect(shown()[0]).toBe('line 456'));

    const real = session.ipc.invoke.bind(session.ipc);
    let release = () => {};
    const gate = new Promise<void>((resolve) => (release = resolve));
    session.ipc.invoke = ((name: never, args: { query?: { backward: boolean } }) =>
      args.query?.backward && sent('view_text').length === 2
        ? gate.then(() => real(name, args as never))
        : real(name, args as never)) as never;

    await press('End');
    await press('PageUp');
    await new Promise((resolve) => setTimeout(resolve, 20));
    release();
    await vi.waitFor(() => expect(shown().at(-1)).toBe('line 999'));

    // Up through where the window before the seek would have been joined to the End's.
    for (let step = 0; step < 8; step += 1) {
      await press('PageUp');
      await new Promise((resolve) => setTimeout(resolve, 10));

      const numbers = shown().map((text) => Number(text?.replace('line ', '')));
      expect(numbers).toEqual(numbers.map((_, index) => numbers[0] + index));
    }
    expect(shown()[0]).toBe('line 648');
  });

  it('asks again for a window the backend refused as busy', async () => {
    let refused = 0;
    const world = await openedSession(file(), undefined, {
      view_text: (args: { query: Parameters<FakeFile['window']>[0] }) => {
        refused += 1;

        return refused === 1
          ? Promise.reject({ code: 'busy', message: 'the viewer is busy' })
          : file().window(args.query);
      },
    });
    render(TextView, { session: world.session });

    await vi.waitFor(() => expect(shown()[0]).toBe('line 0'));
    expect(refused).toBe(2);
  });

  it('wrapping is a style of the view: no request, and the choice is on the view', async () => {
    const { sent, session } = await open();
    expect(screen.getByRole('textbox')).toHaveAttribute('data-wrap', 'false');

    session.wrap = true;

    await vi.waitFor(() => expect(screen.getByRole('textbox')).toHaveAttribute('data-wrap', 'true'));
    expect(sent('view_text')).toHaveLength(1);
  });

  it('another encoding asks again for the line at the top, in that encoding', async () => {
    const { sent, session } = await open();
    await fireEvent.wheel(screen.getByRole('textbox'), { deltaY: 18 * 7 });
    const top = file().starts[7];

    session.encoding = 'windows-1250';

    await vi.waitFor(() => expect(sent('view_text')).toHaveLength(2));
    expect(sent('view_text')[1]).toEqual({
      session: 1,
      query: { encoding: 'windows-1250', offset: top, backward: false, lines: FETCH, snap: true },
    });
    expect(shown()[0]).toBe('line 7');
  });

  it('goes to a match and marks its line', async () => {
    const content = file();
    const { view } = await open(content);
    const hit = { offset: content.starts[500] + 2, len: 3 };

    await view.rerender({ hit });

    await vi.waitFor(() => expect(shown()[0]).toBe('line 500'));
    expect(screen.getByText('line 500')).toHaveAttribute('aria-current', 'true');
    expect(screen.getByText('line 501')).not.toHaveAttribute('aria-current');
  });

  it('shows what went wrong, in the backend`s words', async () => {
    const world = await openedSession(file(), undefined, {
      view_text: () => Promise.reject({ code: 'io', message: 'the viewer cannot read the file' }),
    });
    render(TextView, { session: world.session });

    expect(await screen.findByRole('alert')).toHaveTextContent('the viewer cannot read the file');
  });
});
