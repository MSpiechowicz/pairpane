import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import { FakeFile, openedSession } from '../../../tests/view';
import HexView from './HexView.svelte';

/** 4096 bytes, each the value of its offset modulo 256; the view holds 32 rows and reads 1536 bytes at a time. */
const file = () => new FakeFile(Uint8Array.from({ length: 4096 }, (_, index) => index % 256));
const READ = 32 * 16 * 3;

const rows = () =>
  Array.from(screen.getByRole('textbox').children).map((row) => (row.textContent ?? '').replace(/\s+/g, ' ').trim());
const press = (key: string) => fireEvent.keyDown(screen.getByRole('textbox'), { key });

async function open() {
  const world = await openedSession(file());
  const view = render(HexView, { session: world.session });
  await vi.waitFor(() => expect(rows()).not.toHaveLength(0));
  await vi.waitFor(() => expect(rows()[0]).toMatch(/^00000000 00 01/));

  return { ...world, view };
}

describe('HexView', () => {
  it('shows 16-byte rows: the offset, the bytes in hex and their ASCII', async () => {
    const { sent } = await open();

    expect(rows()[0]).toBe('00000000 00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f ................');
    expect(rows()[2]).toBe('00000020 20 21 22 23 24 25 26 27 28 29 2a 2b 2c 2d 2e 2f !"#$%&\'()*+,-./');
    expect(rows()).toHaveLength(33);
    expect(sent('view_bytes')).toEqual([{ session: 1, offset: 0, length: READ }]);
  });

  it('scrolls by rows from the bytes it holds, and reads more only when the rows in view are not held', async () => {
    const { sent, session } = await open();

    await fireEvent.wheel(screen.getByRole('textbox'), { deltaY: 18 * 5 });
    await press('ArrowDown');
    expect(rows()[0]).toMatch(/^00000060 60 61/);
    expect(sent('view_bytes')).toHaveLength(1);
    expect(session.position).toBe(96);

    for (let step = 0; step < 60; step += 1) {
      await press('ArrowDown');
    }

    await vi.waitFor(() => expect(sent('view_bytes')).toHaveLength(2));
    // The first top whose rows are not all held is row 65; the read is centred on it.
    expect(sent('view_bytes')[1]).toEqual({ session: 1, offset: 65 * 16 - 32 * 16, length: READ });
    await vi.waitFor(() => expect(rows()[0]).toMatch(/^00000420 20 21/));
  });

  it('PageDown moves a page less a row; Home and End go to the first and the last page', async () => {
    const { sent } = await open();

    await press('PageDown');
    expect(rows()[0]).toMatch(/^000001f0/);

    await press('End');
    await vi.waitFor(() => expect(rows()[0]).toMatch(/^00000e00/));
    expect(rows().at(-1)).toMatch(/^00000ff0/);
    expect(sent('view_bytes').at(-1)).toEqual({ session: 1, offset: 0xe00 - 512, length: READ });

    await press('Home');
    await vi.waitFor(() => expect(rows()[0]).toMatch(/^00000000/));
  });

  it('a read for a place the view has left is dropped, also when the view is back on bytes it holds', async () => {
    const { session } = await open();
    const real = session.ipc.invoke.bind(session.ipc);
    let release = () => {};
    const gate = new Promise<void>((resolve) => (release = resolve));
    session.ipc.invoke = ((name: never, args: { offset?: number }) =>
      args.offset === 0xe00 - 512 ? gate.then(() => real(name, args as never)) : real(name, args as never)) as never;

    await press('End');
    await press('Home');
    release();
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(rows()[0]).toMatch(/^00000000 00 01/);
    expect(rows()).toHaveLength(33);
  });

  it('asks again for bytes the backend refused as busy', async () => {
    let refused = 0;
    const world = await openedSession(file(), undefined, {
      view_bytes: (args: { offset: number; length: number }) => {
        refused += 1;

        return refused === 1
          ? Promise.reject({ code: 'busy', message: 'the viewer is busy' })
          : file().bytes.slice(args.offset, args.offset + args.length).buffer;
      },
    });
    render(HexView, { session: world.session });

    await vi.waitFor(() => expect(rows()[0]).toMatch(/^00000000 00 01/));
    expect(refused).toBe(2);
  });

  it('dragging the scroll bar moves to the row of the byte it stands for', async () => {
    const { view } = await open();
    const track = view.container.querySelector<HTMLElement>('.track')!;

    track.scrollTop = 1605;
    await fireEvent.scroll(track);

    await vi.waitFor(() => expect(rows()[0]).toMatch(/^00000640/));
  });

  it('goes to a match and marks its bytes', async () => {
    const { view } = await open();

    await view.rerender({ hit: { offset: 0x6e, len: 4 } });

    await vi.waitFor(() => expect(rows()[0]).toMatch(/^00000060/));
    const marked = screen.getAllByText(/^[0-9a-f]{2}$/).filter((cell) => cell.getAttribute('aria-current') === 'true');
    expect(marked.map((cell) => cell.textContent)).toEqual(['6e', '6f', '70', '71']);
  });
});
