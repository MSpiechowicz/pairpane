import { describe, expect, it, vi } from 'vitest';

import type { TextQuery } from '$lib/ipc/bindings/TextQuery';
import type { TextWindow } from '$lib/ipc/bindings/TextWindow';

import { FakeFile, openedSession } from '../../../tests/view';
import { TextModel } from './textWindow.svelte';

const PIECE = 16 * 1024;
const SIZE = 64 * 1024 * 1024;
const PIECES = SIZE / PIECE;
const ROWS = 40;

/** One line of 64 MB with no newline, which the backend serves in pieces of 16 KiB (each piece's text is its number). */
function hugeLine(query: TextQuery): TextWindow {
  const to = query.backward ? Math.ceil(query.offset / PIECE) : Math.min(PIECES, Math.floor(query.offset / PIECE) + query.lines);
  const from = query.backward ? Math.max(0, to - query.lines) : Math.floor(query.offset / PIECE);
  const lines = Array.from({ length: to - from }, (_, index) => ({
    offset: (from + index) * PIECE,
    text: String(from + index),
    continues: from + index + 1 < PIECES,
  }));

  return { start: from * PIECE, end: Math.min(SIZE, to * PIECE), lines, atStart: from === 0, atEnd: to >= PIECES };
}

async function open() {
  const world = await openedSession(new FakeFile(new Uint8Array(1)), undefined, { view_text: (args: { query: TextQuery }) => hugeLine(args.query) });
  const model = new TextModel(world.session);
  model.rows = ROWS;
  await model.home();

  return model;
}

/** The pieces in the buffer are consecutive ones. */
const contiguous = (model: TextModel) =>
  model.lines.every((line, index) => index === 0 || Number(line.text) === Number(model.lines[index - 1].text) + 1);

/** Scrolls by `step` lines until the view is at `piece`, waiting for the windows on the way. */
async function scrollTo(model: TextModel, piece: number, step: number) {
  while (Number(model.lines[model.first].text) !== piece) {
    model.scrollBy(step);
    await vi.waitFor(() => {
      const after = model.lines.length - (model.first + ROWS);
      expect(step > 0 ? after >= ROWS || model.atEnd : model.first >= ROWS || model.atStart).toBe(true);
    });
    expect(model.lines.length).toBeLessThanOrEqual(600);
    expect(contiguous(model)).toBe(true);
  }
}

describe('TextModel on one huge line', () => {
  it('keeps the buffer bounded, contiguous and right while scrolling down and back up', async () => {
    const model = await open();
    expect(model.lines[0].text).toBe('0');

    await scrollTo(model, 1200, 40);
    expect(model.lines.length).toBeLessThan(1200);
    expect(model.atStart).toBe(false);
    expect(model.atEnd).toBe(false);
    expect(model.lines[model.first].offset).toBe(1200 * PIECE);

    await scrollTo(model, 400, -40);
    expect(model.lines[model.first].offset).toBe(400 * PIECE);
    expect(model.lines.length).toBeLessThanOrEqual(600);
    expect(model.top).toBe(400 * PIECE);
  });
});
