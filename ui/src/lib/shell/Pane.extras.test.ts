import { fireEvent, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { PaneTabs } from '$lib/panes/tabs.svelte';
import { preferences } from '$lib/shell/preferences.svelte';

import { archiveStart, arrive, entry, fakeBackend, resultsStart } from '../../../tests/panes';
import { mountPane } from '../../../tests/pane';
import { pending, refusal, settle } from '../../../tests/thumbs';

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

const HEX = '0123456789abcdef'.repeat(4);

type Answers = NonNullable<Parameters<typeof fakeBackend>[0]>;

const SAMPLE = [entry('sub', 'dir'), entry('a.jpg'), entry('b.txt'), entry('c.PNG'), entry('d.heic')];

/** A pane in the detailed layout over `/pics` (so row 0 is `..`), listed with `entries`. */
async function shown(answers: Answers = {}, entries = SAMPLE) {
  const backend = fakeBackend(answers);
  const tabs = new PaneTabs(backend.ipc, 'left');
  await tabs.view.listing.open('/pics');
  arrive(backend.last(), '/pics', entries);
  await mountPane(backend, tabs);

  return { backend, tabs, view: tabs.view };
}

/** What the backend was asked by `command`, in order. */
const sent = (backend: ReturnType<typeof fakeBackend>, command: string) =>
  backend.invoke.mock.calls.filter(([name]) => name === command).map(([, args]) => args as unknown as Record<string, unknown>);

const headers = () => within(screen.getByRole('group', { name: 'Sort by' })).getAllByRole('button').map((button) => /^\w+/.exec(button.textContent ?? '')?.[0]);
const cells = (kind: 'dm' | 'ck') => [...document.querySelectorAll<HTMLElement>(`.row .${kind}`)];
const rowNames = () => [...document.querySelectorAll('.row')].map((row) => row.querySelector('.nm')?.textContent);

const SIZES: Record<string, { width: number; height: number } | null> = {
  'a.jpg': { width: 640, height: 480 },
  'c.PNG': { width: 1920, height: 1080 },
  'd.heic': null,
};
const dimensions = ({ names }: { names: string[] }) => names.map((name) => SIZES[name] ?? null);

describe('Pane: the extra columns', () => {
  it('has neither column, and asks for nothing, until the settings show them', async () => {
    const { backend } = await shown({ columns_dimensions: dimensions as never, columns_checksum: () => HEX });
    await settle();

    expect(headers()).toEqual(['Name', 'Ext', 'Size', 'Modified']);
    expect(cells('dm')).toHaveLength(0);
    expect(cells('ck')).toHaveLength(0);
    expect(sent(backend, 'columns_dimensions')).toEqual([]);
    expect(sent(backend, 'columns_checksum')).toEqual([]);
  });

  it('shows the Dimensions column after Modified once the setting is on, with the size of the pictures in view', async () => {
    const { backend } = await shown({ columns_dimensions: dimensions as never });

    preferences.set({ showDimensions: true });
    await settle();

    expect(headers()).toEqual(['Name', 'Ext', 'Size', 'Modified', 'Dimensions']);
    expect(sent(backend, 'columns_dimensions')).toEqual([{ path: '/pics', names: ['a.jpg', 'c.PNG', 'd.heic'] }]); // pictures only
    expect(sent(backend, 'columns_checksum')).toEqual([]);
    // The rows sub, a.jpg, b.txt, c.PNG and d.heic (`..` has no cells).
    expect(cells('dm').map((cell) => cell.textContent)).toEqual(['', '640×480', '', '1920×1080', '–']);
  });

  it('puts the cells in the grid after Modified, with a track for each shown column', async () => {
    await shown();

    preferences.set({ showDimensions: true, showChecksum: true });
    await settle();

    const grid = (document.querySelector('.pane') as HTMLElement).style.getPropertyValue('--cols-full');
    expect(grid).toBe('20px minmax(0, 1fr) 48px 74px 118px 92px 104px');
    const row = document.querySelector('[data-row="1"]') as HTMLElement;
    expect([...row.children].map((child) => child.className.split(' ')[0]).slice(-2)).toEqual(['dm', 'ck']);
    preferences.set({ showDimensions: false });
    await settle();
    expect((document.querySelector('.pane') as HTMLElement).style.getPropertyValue('--cols-full')).toBe('20px minmax(0, 1fr) 48px 74px 118px 104px');
  });

  it('carries the size as a tooltip, and a dash with its own for a picture without one', async () => {
    await shown({ columns_dimensions: dimensions as never });

    preferences.set({ showDimensions: true });
    await settle();

    const [, a, , c, d] = cells('dm');
    expect(a).toHaveAttribute('title', '640 × 480 pixels');
    expect(c).toHaveAttribute('title', '1920 × 1080 pixels');
    expect(d).toHaveAttribute('title', 'No dimensions');
  });

  it('shows the first 12 digits of the checksum of each file (not of a folder or `..`) with the whole one as the tooltip', async () => {
    const { backend } = await shown({ columns_checksum: () => HEX });

    preferences.set({ showChecksum: true });
    await settle();

    expect(headers()).toEqual(['Name', 'Ext', 'Size', 'Modified', 'Checksum']);
    expect(sent(backend, 'columns_checksum').map((ask) => ask.name)).toEqual(['a.jpg', 'b.txt', 'c.PNG', 'd.heic']);
    expect(sent(backend, 'columns_checksum')[0]).toEqual({ path: '/pics', name: 'a.jpg' });
    expect(cells('ck').map((cell) => cell.textContent)).toEqual(['', ...Array(4).fill(HEX.slice(0, 12))]);
    expect(cells('ck')[1]).toHaveAttribute('title', HEX);
  });

  it('has two checksums asked for at once at the most', async () => {
    const answers = new Map<string, ReturnType<typeof pending>>();
    const { backend } = await shown({
      columns_checksum: (({ name }: { name: string }) => {
        const answer = pending();
        answers.set(name, answer);
        return answer.answer;
      }) as never,
    });

    preferences.set({ showChecksum: true });
    await settle();
    expect(sent(backend, 'columns_checksum').map((ask) => ask.name)).toEqual(['a.jpg', 'b.txt']);

    answers.get('a.jpg')?.resolve(HEX as never);
    await settle();

    expect(sent(backend, 'columns_checksum').map((ask) => ask.name)).toEqual(['a.jpg', 'b.txt', 'c.PNG']);
  });

  it('shows a dash with the reason for a file too big to checksum', async () => {
    await shown({
      columns_checksum: (() => Promise.reject({ code: 'invalidInput', message: 'the file is too big to checksum here' })) as never,
    });

    preferences.set({ showChecksum: true });
    await settle();

    const [, a] = cells('ck');
    expect(a).toHaveTextContent('–');
    expect(a).toHaveAttribute('title', 'the file is too big to checksum here');
  });

  it('asks again later when the backend is busy, and the cell fills in', async () => {
    vi.useFakeTimers();
    let refusals = 1;
    const { backend } = await shown({
      columns_checksum: (() => {
        if (refusals > 0) {
          refusals -= 1;
          return Promise.reject(refusal('busy'));
        }

        return Promise.resolve(HEX);
      }) as never,
    });

    preferences.set({ showChecksum: true });
    await vi.advanceTimersByTimeAsync(0);
    expect(sent(backend, 'columns_checksum').map((ask) => ask.name)).toEqual(['a.jpg', 'b.txt']); // a is refused: the rest waits with it

    await vi.advanceTimersByTimeAsync(300);
    await tick();

    expect(sent(backend, 'columns_checksum').map((ask) => ask.name)).toEqual(['a.jpg', 'b.txt', 'a.jpg', 'c.PNG', 'd.heic']);
    expect(cells('ck').map((cell) => cell.textContent)).toEqual(['', ...Array(4).fill(HEX.slice(0, 12))]);
  });

  it('asks for nothing in the brief and thumbnails layouts, and shows no column in them', async () => {
    const { backend, view } = await shown({ columns_dimensions: dimensions as never, columns_checksum: () => HEX });
    view.layout = 'brief';
    preferences.set({ showDimensions: true, showChecksum: true });
    await settle();

    expect(sent(backend, 'columns_dimensions')).toEqual([]);
    expect(sent(backend, 'columns_checksum')).toEqual([]);
    expect(cells('dm')).toHaveLength(0);
    view.layout = 'thumbnails';
    await settle();
    expect(sent(backend, 'columns_checksum')).toEqual([]);

    view.layout = 'detailed';
    await settle();
    expect(sent(backend, 'columns_dimensions')).toHaveLength(1);
  });

  it('stops asking when a column is turned off', async () => {
    const answers = new Map<string, ReturnType<typeof pending>>();
    const { backend } = await shown({
      columns_checksum: (({ name }: { name: string }) => {
        const answer = pending();
        answers.set(name, answer);
        return answer.answer;
      }) as never,
    });
    preferences.set({ showChecksum: true });
    await settle();
    expect(sent(backend, 'columns_checksum')).toHaveLength(2);

    preferences.set({ showChecksum: false });
    await settle();
    answers.get('a.jpg')?.resolve(HEX as never);
    await settle();

    expect(sent(backend, 'columns_checksum')).toHaveLength(2);
    expect(cells('ck')).toHaveLength(0);
  });

  it('asks for nothing inside an archive', async () => {
    const backend = fakeBackend({ columns_dimensions: dimensions as never, columns_checksum: () => HEX });
    const tabs = new PaneTabs(backend.ipc, 'left');
    await tabs.view.listing.openArchive({ archive: '/pics/a.zip', dir: '' });
    backend.last().emit(archiveStart('/pics', 2, { archive: '/pics/a.zip', dir: '' }));
    backend.last().emit({ kind: 'batch', offset: 0, entries: [entry('a.jpg'), entry('b.txt')] });
    await mountPane(backend, tabs);

    preferences.set({ showDimensions: true, showChecksum: true });
    await settle();

    expect(cells('dm').map((cell) => cell.textContent)).toEqual(['', '']); // a.jpg and b.txt
    expect(sent(backend, 'columns_dimensions')).toEqual([]);
    expect(sent(backend, 'columns_checksum')).toEqual([]);
  });

  it('asks in the folder that holds an item of a search result', async () => {
    const backend = fakeBackend({ columns_dimensions: dimensions as never });
    const tabs = new PaneTabs(backend.ipc, 'left');
    await tabs.view.listing.openResults(7);
    backend.last().emit(resultsStart('/home/me', 3, 7));
    backend.last().emit({ kind: 'batch', offset: 0, entries: [entry('docs/a.jpg'), entry('top.png'), entry('docs/more/c.PNG')] });
    await mountPane(backend, tabs);

    preferences.set({ showDimensions: true });
    await settle();

    expect(sent(backend, 'columns_dimensions')).toEqual([
      { path: '/home/me/docs', names: ['a.jpg'] },
      { path: '/home/me', names: ['top.png'] },
      { path: '/home/me/docs/more', names: ['c.PNG'] },
    ]);
  });

  it('drops what was queued when the folder changes', async () => {
    const answers = new Map<string, ReturnType<typeof pending>>();
    const { backend, tabs } = await shown({
      columns_checksum: (({ name }: { name: string }) => {
        const answer = pending();
        answers.set(name, answer);
        return answer.answer;
      }) as never,
    });
    preferences.set({ showChecksum: true });
    await settle();
    expect(sent(backend, 'columns_checksum').map((ask) => ask.name)).toEqual(['a.jpg', 'b.txt']);

    await tabs.view.listing.open('/other');
    arrive(backend.last(), '/other', [entry('z.bin')]);
    await settle();
    answers.get('a.jpg')?.resolve(HEX as never);
    await settle();

    expect(sent(backend, 'columns_checksum').map((ask) => ask.name)).toEqual(['a.jpg', 'b.txt', 'z.bin']);
  });
});

describe('Pane: sorting by the extra columns', () => {
  it('sorts by pixel count from the Dimensions header, the pictures it knows nothing of last, folders on top', async () => {
    await shown({ columns_dimensions: dimensions as never });
    preferences.set({ showDimensions: true });
    await settle();

    await fireEvent.click(screen.getByRole('button', { name: 'Dimensions' }));
    expect(rowNames()).toEqual(['..', 'sub', 'a.jpg', 'c.PNG', 'b.txt', 'd.heic']); // b.txt is no picture: unknown
    expect(screen.getByRole('button', { name: /Dimensions\s*sorted ascending/ })).toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: /Dimensions/ }));
    expect(rowNames()).toEqual(['..', 'sub', 'c.PNG', 'a.jpg', 'b.txt', 'd.heic']);
  });

  it('sorts by the checksum from the Checksum header, and by what has arrived so far', async () => {
    const sums: Record<string, string> = { 'a.jpg': 'f'.repeat(64), 'b.txt': 'a'.repeat(64), 'c.PNG': 'c'.repeat(64) };
    await shown({ columns_checksum: (({ name }: { name: string }) => sums[name] ?? Promise.reject(refusal('busy'))) as never });
    preferences.set({ showChecksum: true });
    await settle();

    await fireEvent.click(screen.getByRole('button', { name: 'Checksum' }));

    expect(rowNames()).toEqual(['..', 'sub', 'b.txt', 'c.PNG', 'a.jpg', 'd.heic']); // d.heic is not known (busy)
  });
});
