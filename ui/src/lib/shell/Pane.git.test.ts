import { cleanup, fireEvent, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { GitMark } from '$lib/ipc/bindings/GitMark';
import type { GitStatus } from '$lib/ipc/bindings/GitStatus';
import { GIT_TTL_MS } from '$lib/panes/git.svelte';
import { PaneTabs } from '$lib/panes/tabs.svelte';
import { preferences } from '$lib/shell/preferences.svelte';

import { mountPane } from '../../../tests/pane';
import { archiveStart, arrive, entry, fakeBackend, resultsStart } from '../../../tests/panes';
import { refusal, settle } from '../../../tests/thumbs';

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

type Answers = NonNullable<Parameters<typeof fakeBackend>[0]>;

const SAMPLE = [entry('sub', 'dir'), entry('a.txt'), entry('b.txt'), entry('c.txt'), entry('d.txt')];

const repo = (...marks: [string, GitMark][]): GitStatus => ({ repo: true, marks: marks.map(([name, mark]) => ({ name, mark })) });
const MARKED = repo(['sub', 'untracked'], ['a.txt', 'modified'], ['b.txt', 'conflicted'], ['c.txt', 'staged']);

/** A pane in the detailed layout over `/proj` (so row 0 is `..`), listed with `entries`. */
async function shown(answers: Answers = {}, entries = SAMPLE) {
  const backend = fakeBackend(answers);
  const tabs = new PaneTabs(backend.ipc, 'left');
  await tabs.view.listing.open('/proj');
  arrive(backend.last(), '/proj', entries);
  await mountPane(backend, tabs);

  return { backend, tabs, view: tabs.view };
}

/** The folders the backend was asked the git status of, in order. */
const asked = (backend: ReturnType<typeof fakeBackend>) =>
  backend.invoke.mock.calls.filter(([name]) => name === 'columns_git').map(([, args]) => (args as unknown as { path: string }).path);

const headers = () => within(screen.getByRole('group', { name: 'Sort by' })).getAllByRole('button').map((button) => /^\w+/.exec(button.textContent ?? '')?.[0]);
const marks = () => [...document.querySelectorAll<HTMLElement>('.row .gt')];
const rowNames = () => [...document.querySelectorAll('.row')].map((row) => row.querySelector('.nm')?.textContent);

/** An answer the test settles when it likes. */
const later = () => {
  let resolve: (value: unknown) => void = () => {};
  let reject: (reason: unknown) => void = () => {};
  const answer = new Promise((ok, no) => {
    resolve = ok;
    reject = no;
  });

  return { answer, resolve, reject };
};

describe('Pane: the Git column', () => {
  it('is not there, and asks for nothing, until the settings show it', async () => {
    const { backend } = await shown({ columns_git: () => MARKED });
    await settle();

    expect(headers()).toEqual(['Name', 'Ext', 'Size', 'Modified']);
    expect(marks()).toHaveLength(0);
    expect(asked(backend)).toEqual([]);
  });

  it('comes after Modified once the setting is on, goes when it is off, and asks no more then', async () => {
    const { backend } = await shown({ columns_git: () => MARKED });

    preferences.set({ showGit: true });
    await settle();
    expect(headers()).toEqual(['Name', 'Ext', 'Size', 'Modified', 'Git']);
    expect((document.querySelector('.pane') as HTMLElement).style.getPropertyValue('--cols-full')).toBe('20px minmax(0, 1fr) 48px 74px 118px 44px');

    preferences.set({ showGit: false });
    await settle();
    expect(headers()).toEqual(['Name', 'Ext', 'Size', 'Modified']);
    expect(marks()).toHaveLength(0);
    expect(asked(backend)).toHaveLength(1);
  });

  it('asks once for the folder, whatever the number of rows', async () => {
    const { backend } = await shown({ columns_git: () => MARKED });

    preferences.set({ showGit: true });
    await settle();

    expect(asked(backend)).toEqual(['/proj']);
  });

  it('shows a letter for each mark, a folder included, with the words as the tooltip, and nothing for a clean item', async () => {
    await shown({ columns_git: () => MARKED });

    preferences.set({ showGit: true });
    await settle();

    // The rows sub, a.txt, b.txt, c.txt and d.txt (`..` has no cell).
    expect(marks().map((cell) => cell.textContent)).toEqual(['?', 'M', 'U', 'S', '']);
    expect(marks().map((cell) => cell.getAttribute('title')?.split(':')[0] ?? null)).toEqual(['Untracked', 'Modified', 'Conflicted', 'Staged', null]);
    // The mark of a folder tells about what is inside it; that of a file, about the file.
    expect(marks()[0].getAttribute('title')).toMatch(/what is inside it/);
    expect(marks()[1].getAttribute('title')).not.toMatch(/inside/);
    expect(marks()[2]).toHaveClass('danger');
    expect(marks()[1]).toHaveClass('accent');
  });

  it('shows no marks for a folder outside a repository', async () => {
    const { backend } = await shown({ columns_git: () => ({ repo: false, marks: [] }) });

    preferences.set({ showGit: true });
    await settle();

    expect(asked(backend)).toEqual(['/proj']);
    expect(marks().map((cell) => cell.textContent)).toEqual(['', '', '', '', '']);
  });

  it('keeps the answer while nothing changes, and asks again when the folder is listed again', async () => {
    let status = MARKED;
    const { backend, tabs } = await shown({ columns_git: () => status });
    preferences.set({ showGit: true });
    await settle();

    await settle();
    expect(asked(backend)).toHaveLength(1);

    status = repo(['d.txt', 'modified']);
    await tabs.view.listing.reload();
    arrive(backend.last(), '/proj', SAMPLE);
    await settle();

    expect(asked(backend)).toHaveLength(2);
    expect(marks().map((cell) => cell.textContent)).toEqual(['', '', '', '', 'M']);
  });

  it('asks again about 10 seconds later, so an edit in an editor shows up', async () => {
    vi.useFakeTimers();
    let status = repo(['a.txt', 'modified']);
    const { backend } = await shown({ columns_git: () => status });
    preferences.set({ showGit: true });
    await vi.advanceTimersByTimeAsync(0);
    expect(marks().map((cell) => cell.textContent)).toEqual(['', 'M', '', '', '']);

    status = repo(['a.txt', 'staged']);
    await vi.advanceTimersByTimeAsync(GIT_TTL_MS - 1);
    expect(asked(backend)).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    await tick();

    expect(asked(backend)).toHaveLength(2);
    expect(marks().map((cell) => cell.textContent)).toEqual(['', 'S', '', '', '']);
  });

  it('stops asking, and leaves no timer, once the pane is gone', async () => {
    vi.useFakeTimers();
    const { backend } = await shown({ columns_git: () => MARKED });
    preferences.set({ showGit: true });
    await vi.advanceTimersByTimeAsync(0);
    expect(asked(backend)).toHaveLength(1);

    cleanup();
    await vi.advanceTimersByTimeAsync(GIT_TTL_MS * 3);

    expect(asked(backend)).toHaveLength(1);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('asks again later when the backend is busy, and the marks fill in', async () => {
    vi.useFakeTimers();
    let refusals = 1;
    const { backend } = await shown({
      columns_git: (() => {
        if (refusals > 0) {
          refusals -= 1;
          return Promise.reject(refusal('busy'));
        }

        return Promise.resolve(MARKED);
      }) as never,
    });

    preferences.set({ showGit: true });
    await vi.advanceTimersByTimeAsync(0);
    expect(asked(backend)).toHaveLength(1);
    expect(marks().map((cell) => cell.textContent)).toEqual(['', '', '', '', '']);

    await vi.advanceTimersByTimeAsync(300);
    await tick();

    expect(asked(backend)).toHaveLength(2);
    expect(marks().map((cell) => cell.textContent)).toEqual(['?', 'M', 'U', 'S', '']);
  });

  it('shows a dash with the reason in each row of a repository too big to read, and does not ask again', async () => {
    vi.useFakeTimers();
    const { backend } = await shown({
      columns_git: (() => Promise.reject({ code: 'invalidInput', message: 'the repository is too big to read here' })) as never,
    });

    preferences.set({ showGit: true });
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(GIT_TTL_MS * 2);
    await tick();

    expect(marks().map((cell) => cell.textContent)).toEqual(['–', '–', '–', '–', '–']);
    expect(marks()[0]).toHaveAttribute('title', 'the repository is too big to read here');
    expect(asked(backend)).toHaveLength(1);
  });

  it('asks for nothing in the brief and thumbnails layouts, and shows no column in them', async () => {
    const { backend, view } = await shown({ columns_git: () => MARKED });
    view.layout = 'brief';
    preferences.set({ showGit: true });
    await settle();

    expect(asked(backend)).toEqual([]);
    expect(marks()).toHaveLength(0);
    view.layout = 'thumbnails';
    await settle();
    expect(asked(backend)).toEqual([]);

    view.layout = 'detailed';
    await settle();
    expect(asked(backend)).toEqual(['/proj']);
  });

  it('asks for nothing inside an archive', async () => {
    const backend = fakeBackend({ columns_git: () => MARKED });
    const tabs = new PaneTabs(backend.ipc, 'left');
    await tabs.view.listing.openArchive({ archive: '/proj/a.zip', dir: '' });
    backend.last().emit(archiveStart('/proj', 2, { archive: '/proj/a.zip', dir: '' }));
    backend.last().emit({ kind: 'batch', offset: 0, entries: [entry('a.txt'), entry('b.txt')] });
    await mountPane(backend, tabs);

    preferences.set({ showGit: true });
    await settle();

    expect(marks().map((cell) => cell.textContent)).toEqual(['', '']);
    expect(asked(backend)).toEqual([]);
  });

  it('asks about each folder that holds an item of a search result, one at a time, and shows the marks of each', async () => {
    const answers = new Map<string, ReturnType<typeof later>>();
    const backend = fakeBackend({
      columns_git: (({ path }: { path: string }) => {
        const answer = later();
        answers.set(path, answer);
        return answer.answer;
      }) as never,
    });
    const tabs = new PaneTabs(backend.ipc, 'left');
    await tabs.view.listing.openResults(7);
    backend.last().emit(resultsStart('/home/me', 3, 7));
    backend.last().emit({ kind: 'batch', offset: 0, entries: [entry('docs/a.txt'), entry('top.txt'), entry('docs/more/c.txt')] });
    await mountPane(backend, tabs);

    preferences.set({ showGit: true });
    await settle();
    expect(asked(backend)).toEqual(['/home/me/docs']);

    answers.get('/home/me/docs')?.resolve(repo(['a.txt', 'modified']));
    await settle();
    expect(asked(backend)).toEqual(['/home/me/docs', '/home/me']);

    answers.get('/home/me')?.resolve(repo(['top.txt', 'untracked']));
    await settle();
    answers.get('/home/me/docs/more')?.resolve({ repo: false, marks: [] });
    await settle();
    expect(asked(backend)).toEqual(['/home/me/docs', '/home/me', '/home/me/docs/more']);

    expect(marks().map((cell) => cell.textContent)).toEqual(['M', '?', '']);
  });

  it('sorts by the seriousness of the mark from the Git header, the items without one last, folders on top', async () => {
    await shown({ columns_git: () => MARKED });
    preferences.set({ showGit: true });
    await settle();

    await fireEvent.click(screen.getByRole('button', { name: 'Git' }));
    expect(rowNames()).toEqual(['..', 'sub', 'b.txt', 'a.txt', 'c.txt', 'd.txt']); // conflicted, modified, staged, clean
    expect(screen.getByRole('button', { name: /Git\s*sorted ascending/ })).toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: /Git/ }));
    expect(rowNames()).toEqual(['..', 'sub', 'c.txt', 'a.txt', 'b.txt', 'd.txt']);
  });
});
