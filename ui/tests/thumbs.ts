// Test helpers for the thumbnails: the backend's answer to `thumb_get`, a request the test settles when it likes, and a
// pane in the thumbnails layout.
import { render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { vi } from 'vitest';

import { Hotlist } from '$lib/panes/hotlist.svelte';
import { PaneTabs } from '$lib/panes/tabs.svelte';
import { Volumes } from '$lib/panes/volumes.svelte';

import Pane from '../src/lib/shell/Pane.svelte';
import { entry, fakeBackend } from './panes';

/** A `thumb_get` answer: the width and height (u32, little-endian), then `width * height` RGBA pixels (all of `pixel`, or as given). */
export function picture(width: number, height: number, pixels?: ArrayLike<number>, pixel = [10, 20, 30, 255]): ArrayBuffer {
  const bytes = new Uint8Array(8 + width * height * 4);
  const header = new DataView(bytes.buffer);
  header.setUint32(0, width, true);
  header.setUint32(4, height, true);

  for (let at = 0; at < width * height * 4; at += 1) {
    bytes[8 + at] = pixels ? pixels[at] : pixel[at % 4];
  }

  return bytes.buffer;
}

/** An answer the test settles later: `settle` resolves it with a picture, or rejects it as the backend does. */
export function pending() {
  let resolve: (value: ArrayBuffer) => void = () => {};
  let reject: (reason: unknown) => void = () => {};
  const answer = new Promise<ArrayBuffer>((ok, no) => {
    resolve = ok;
    reject = no;
  });

  return { answer, resolve, reject };
}

/** What the backend refuses with. */
export const refusal = (code: string) => ({ code, message: `the backend says ${code}` });

// A pane in the thumbnails layout.

export const TOTAL = 100_000;
export const ACROSS = 3;
export const ROWS = 3;
export const TILE_WIDTH = 136;
export const TILE_HEIGHT = 172;

type Answers = NonNullable<Parameters<typeof fakeBackend>[0]>;

/** Shows `tabs` in a pane, in the thumbnails layout. */
export async function mountThumbnails(backend: ReturnType<typeof fakeBackend>, tabs: PaneTabs) {
  tabs.view.layout = 'thumbnails';
  render(Pane, {
    label: 'Left pane',
    tabs,
    volumes: new Volumes(backend.ipc),
    hotlist: new Hotlist(),
    menu: undefined,
    active: true,
    onActivate: vi.fn(),
    onMenu: vi.fn(),
  });
  await tick();
}

const PICTURES = [entry('sub', 'dir'), ...Array.from({ length: 199 }, (_, i) => entry(`p${i}.jpg`, 'file', { size: 10 }))];

/**
 * A pane in the thumbnails layout over `/big`: by default a folder of 100 000 entries (only 200 have arrived), `sub`
 * and then `p0.jpg`, `p1.jpg`… Row 0 is `..`, row 1 `sub`, row 2 `p0.jpg`. `thumb_get` is answered as `answers` says.
 */
export async function thumbnails(answers: Answers = {}, entries = PICTURES) {
  const backend = fakeBackend(answers);
  const tabs = new PaneTabs(backend.ipc, 'left');
  await tabs.view.listing.open('/big');
  backend.last().emit({
    kind: 'start',
    path: '/big',
    parent: '/',
    crumbs: [
      { name: '/', path: '/' },
      { name: 'big', path: '/big' },
    ],
    total: entries === PICTURES ? TOTAL : entries.length,
  });
  backend.last().emit({ kind: 'batch', offset: 0, entries });
  await mountThumbnails(backend, tabs);

  return { backend, tabs, view: tabs.view };
}

/** A list that holds 3 rows of 3 tiles (the test DOM measures nothing). */
export function sized() {
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(ROWS * TILE_HEIGHT);
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(ACROSS * TILE_WIDTH);
}

export const list = () => screen.getByRole('listbox');
export const viewport = () => document.querySelector('.viewport') as HTMLElement;
export const cursorTile = () => document.querySelector('.row.cursor') as HTMLElement;
export const tileAt = (index: number) => document.querySelector(`[data-row="${index}"]`) as HTMLElement;
export const renderedTiles = () => [...document.querySelectorAll<HTMLElement>('.row')].map((row) => Number(row.dataset.row));

/** The names `thumb_get` was asked for, in order. */
export const asked = (backend: ReturnType<typeof fakeBackend>) =>
  backend.invoke.mock.calls.filter(([name]) => name === 'thumb_get').map(([, args]) => (args as unknown as { name: string }).name);

/** Lets the requests and answers in flight be taken in. */
export async function settle() {
  await new Promise((resolve) => setTimeout(resolve, 0));
  await tick();
}

/** Answers `thumb_get` with a promise per name that the test settles. */
export function held() {
  const answers = new Map<string, ReturnType<typeof pending>>();
  const thumb_get = ({ name }: { name: string }) => {
    const answer = pending();
    answers.set(name, answer);
    return answer.answer;
  };

  return { answers, thumb_get: thumb_get as never };
}
