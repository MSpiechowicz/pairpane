import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { GitMark } from '$lib/ipc/bindings/GitMark';
import type { GitStatus } from '$lib/ipc/bindings/GitStatus';
import type { Transport } from '$lib/ipc/transport';

import { refusal } from '../../../tests/thumbs';
import { MAX_BUSY_IN_A_ROW } from './extrasLane';
import { GIT_TTL_MS, GIT_UNAVAILABLE, GitFolders, UNREADABLE } from './git.svelte';

const place = (name: string, folder = '/repo') => ({ folder, name });
const repo = (...marks: [string, GitMark][]): GitStatus => ({ repo: true, marks: marks.map(([name, mark]) => ({ name, mark })) });
const OUTSIDE: GitStatus = { repo: false, marks: [] };
const TOO_BIG = { code: 'invalidInput', message: 'the repository is too big to read here' };

/** A `GitFolders` over a transport answered by `answer`; `asked` lists the folders it was asked about, in order. */
function world(answer: (path: string) => unknown, options?: ConstructorParameters<typeof GitFolders>[1]) {
  const asked: string[] = [];
  const invoke = vi.fn(async (command: string, { path }: { path: string }) => {
    expect(command).toBe('columns_git');
    asked.push(path);
    return answer(path);
  });

  return { git: new GitFolders({ invoke } as unknown as Transport, options), asked };
}

/** A request the test settles when it likes. */
const held = () => {
  const out = new Map<string, { resolve: (value: unknown) => void; reject: (reason: unknown) => void }>();
  const answer = (path: string) => new Promise((resolve, reject) => out.set(path, { resolve, reject }));

  return { out, answer };
};

/** Lets the answers that are ready be taken in. */
const settled = () => vi.advanceTimersByTimeAsync(0);

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('GitFolders', () => {
  it('asks once for a folder, and the answer gives each item its mark with the words for it', async () => {
    const { git, asked } = world(() =>
      repo(['m.txt', 'modified'], ['s.txt', 'staged'], ['new.txt', 'untracked'], ['out', 'ignored'], ['both.txt', 'conflicted']),
    );

    git.want(['/repo'], 0);
    await settled();

    expect(asked).toEqual(['/repo']);
    expect(git.cell(place('m.txt'))).toMatchObject({ text: 'M', tone: 'accent' });
    expect(git.cell(place('m.txt')).tip).toMatch(/^Modified/);
    expect(git.cell(place('s.txt'))).toMatchObject({ text: 'S', tone: 'ok' });
    expect(git.cell(place('s.txt')).tip).toMatch(/^Staged/);
    expect(git.cell(place('new.txt'))).toMatchObject({ text: '?', tone: 'dim' });
    expect(git.cell(place('new.txt')).tip).toMatch(/^Untracked/);
    expect(git.cell(place('out'))).toMatchObject({ text: '!', tone: 'dim' });
    expect(git.cell(place('out')).tip).toMatch(/^Ignored/);
    expect(git.cell(place('both.txt'))).toMatchObject({ text: 'U', tone: 'danger' });
    expect(git.cell(place('both.txt')).tip).toMatch(/^Conflicted/);
  });

  it('shows nothing for a clean item, for a folder not answered yet, and for an item with no place', async () => {
    const answers = held();
    const { git } = world(answers.answer);

    git.want(['/repo'], 0);
    await settled();
    expect(git.cell(place('a.txt'))).toEqual({ text: '' });

    answers.out.get('/repo')?.resolve(repo(['m.txt', 'modified']));
    await settled();

    expect(git.cell(place('clean.txt'))).toEqual({ text: '' });
    expect(git.cell(place('m.txt', '/elsewhere'))).toEqual({ text: '' });
    expect(git.cell(undefined)).toEqual({ text: '' });
  });

  it('shows nothing for a folder outside a repository, and keeps that answer', async () => {
    const { git, asked } = world(() => OUTSIDE);

    git.want(['/plain'], 0);
    await settled();
    git.want(['/plain'], 0);
    await settled();

    expect(git.cell(place('a.txt', '/plain'))).toEqual({ text: '' });
    expect(asked).toEqual(['/plain']);
  });

  it('keeps the answer for a folder until it is reloaded, and shows the old marks while the new ones come', async () => {
    const answers = held();
    const { git, asked } = world(answers.answer);

    git.want(['/repo'], 0);
    await settled();
    answers.out.get('/repo')?.resolve(repo(['a.txt', 'modified']));
    await settled();
    git.want(['/repo'], 0);
    await settled();
    expect(asked).toEqual(['/repo']);

    git.want(['/repo'], 1); // the listing was reloaded: something changed on disk
    await settled();
    expect(asked).toEqual(['/repo', '/repo']);
    expect(git.cell(place('a.txt')).text).toBe('M');

    answers.out.get('/repo')?.resolve(repo(['a.txt', 'staged']));
    await settled();
    expect(git.cell(place('a.txt')).text).toBe('S');
  });

  it('asks again by itself once the answer is about 10 seconds old, not before', async () => {
    let marks = repo(['a.txt', 'modified']);
    const { git, asked } = world(() => marks);

    git.want(['/repo'], 0);
    await settled();
    marks = repo(['a.txt', 'staged']);

    await vi.advanceTimersByTimeAsync(GIT_TTL_MS - 1);
    expect(asked).toHaveLength(1);
    expect(git.cell(place('a.txt')).text).toBe('M');

    await vi.advanceTimersByTimeAsync(1);
    expect(asked).toHaveLength(2);
    expect(git.cell(place('a.txt')).text).toBe('S');
  });

  it('does not ask again after the time for a folder that is not wanted any more', async () => {
    const { git, asked } = world(() => repo());

    git.want(['/repo'], 0);
    await settled();
    git.want([], 0);
    await vi.advanceTimersByTimeAsync(GIT_TTL_MS * 3);

    expect(asked).toEqual(['/repo']);
  });

  it('asks again for a folder the pane wants once more after the time has passed', async () => {
    const { git, asked } = world(() => repo());

    git.want(['/repo'], 0);
    await settled();
    git.want([], 0);
    await vi.advanceTimersByTimeAsync(GIT_TTL_MS + 1);
    git.want(['/repo'], 0);
    await settled();

    expect(asked).toEqual(['/repo', '/repo']);
  });

  it('asks about the folders of a search result one request each, one at a time, in the order wanted', async () => {
    const answers = held();
    const { git, asked } = world(answers.answer);

    git.want(['/a', '/b', '/c'], 0);
    await settled();
    expect(asked).toEqual(['/a']);

    answers.out.get('/a')?.resolve(repo(['x', 'modified']));
    await settled();
    expect(asked).toEqual(['/a', '/b']);

    answers.out.get('/b')?.resolve(OUTSIDE);
    await settled();
    answers.out.get('/c')?.resolve(repo(['y', 'untracked']));
    await settled();
    expect(asked).toEqual(['/a', '/b', '/c']);
    expect(git.cell(place('x', '/a')).text).toBe('M');
    expect(git.cell(place('y', '/c')).text).toBe('?');
    expect(git.cell(place('x', '/b')).text).toBe('');
  });

  it('drops a folder that was queued and is not wanted any more, and keeps the answer of one that was out', async () => {
    const answers = held();
    const { git, asked } = world(answers.answer);

    git.want(['/a', '/b', '/c'], 0);
    await settled();
    git.want(['/b', '/d'], 0); // /c is gone before its turn
    await settled();

    answers.out.get('/a')?.resolve(repo(['x', 'modified']));
    await settled();
    answers.out.get('/b')?.resolve(OUTSIDE);
    await settled();

    expect(asked).toEqual(['/a', '/b', '/d']);
    expect(git.cell(place('x', '/a')).text).toBe('M');
  });

  it('waits when the backend is busy, longer each time, and asks again', async () => {
    let busy = 2;
    const { git, asked } = world(() => {
      if (busy > 0) {
        busy -= 1;
        return Promise.reject(refusal('busy'));
      }

      return repo(['a.txt', 'modified']);
    });

    git.want(['/repo'], 0);
    await settled();
    expect(asked).toHaveLength(1);

    await vi.advanceTimersByTimeAsync(299);
    expect(asked).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1); // 300 ms after the first refusal
    expect(asked).toHaveLength(2);

    await vi.advanceTimersByTimeAsync(599);
    expect(asked).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(1); // twice as long after the second
    expect(asked).toHaveLength(3);
    expect(git.cell(place('a.txt')).text).toBe('M');
  });

  it(`never gives up on a busy backend: after ${MAX_BUSY_IN_A_ROW} busy answers in a row it asks every 5 seconds, and the marks appear`, async () => {
    let refusals = MAX_BUSY_IN_A_ROW + 1;
    const times: number[] = [];
    const { git, asked } = world(() => {
      times.push(Date.now());
      if (refusals > 0) {
        refusals -= 1;
        return Promise.reject(refusal('busy'));
      }

      return repo(['a.txt', 'modified']);
    });

    git.want(['/repo'], 0);
    await vi.advanceTimersByTimeAsync(29_299); // 300 + 600 + 1200 + 2400 + 4800 ms, then 5 seconds each time, less one

    expect(asked).toHaveLength(MAX_BUSY_IN_A_ROW + 1);
    await vi.advanceTimersByTimeAsync(1);
    expect(asked).toHaveLength(MAX_BUSY_IN_A_ROW + 2);
    expect(git.cell(place('a.txt')).text).toBe('M');

    const gaps = times.slice(1).map((time, index) => time - times[index]);
    expect(gaps.slice(0, 5)).toEqual([300, 600, 1200, 2400, 4800]);
    expect(gaps.slice(5)).toEqual(Array(gaps.length - 5).fill(5000));
  });

  it('asks one request at a time by default, so two panes at most fill the backend', async () => {
    const answers = held();
    const { git, asked } = world(answers.answer);

    git.want(['/a', '/b'], 0);
    await settled();

    expect(asked).toEqual(['/a']);
  });

  it('shows a dash with the reason for a repository too big to read, and asks again after a minute or when the listing is reloaded', async () => {
    let fail = true;
    const { git, asked } = world(() => (fail ? Promise.reject(TOO_BIG) : repo(['a.txt', 'modified'])));

    git.want(['/repo'], 0);
    await settled();
    expect(git.cell(place('a.txt'))).toEqual({ text: '–', tip: 'the repository is too big to read here' });

    git.want(['/repo'], 0);
    await vi.advanceTimersByTimeAsync(GIT_TTL_MS * 3);
    expect(asked).toEqual(['/repo']);

    git.want(['/repo'], 1);
    await settled();
    expect(asked).toEqual(['/repo', '/repo']);

    fail = false;
    await vi.advanceTimersByTimeAsync(60_000 - 1);
    expect(asked).toHaveLength(2);
    expect(git.cell(place('a.txt')).text).toBe('–'); // what was kept shows until the new answer comes
    await vi.advanceTimersByTimeAsync(1);

    expect(asked).toHaveLength(3);
    expect(git.cell(place('a.txt')).text).toBe('M');
  });

  it('shows a dash with the note for a repository that is refused for what it names, and asks again after a minute or when the listing is reloaded', async () => {
    const note = "This repository links git's own files elsewhere, which the app does not follow";
    const { git, asked } = world(() => ({ repo: false, marks: [], note }));

    git.want(['/repo'], 0);
    await settled();
    expect(git.cell(place('a.txt'))).toEqual({ text: '–', tip: note });

    await vi.advanceTimersByTimeAsync(GIT_TTL_MS * 3);
    expect(asked).toEqual(['/repo']);

    git.want(['/repo'], 1);
    await settled();
    expect(asked).toEqual(['/repo', '/repo']);
  });

  it('shows a dash with a fixed text for a repository that cannot be read here, and asks about it again once a minute at most, a reload or not', async () => {
    let fail = true;
    const { git, asked } = world(() => (fail ? Promise.reject(refusal('io')) : repo(['a.txt', 'modified'])));

    git.want(['/repo'], 0);
    await settled();
    expect(git.cell(place('a.txt'))).toEqual({ text: '–', tip: UNREADABLE });
    expect(UNREADABLE).toBe('The repository cannot be read here');

    // A listing that is reloaded again and again (files that change in it) does not make the helper start each time.
    for (let reloads = 1; reloads <= 5; reloads += 1) {
      git.want(['/repo'], reloads);
      await vi.advanceTimersByTimeAsync(10_000);
    }
    expect(asked).toEqual(['/repo']);

    fail = false;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(asked).toHaveLength(2);
    expect(git.cell(place('a.txt')).text).toBe('M');
  });

  it('says there is no git status where the backend has none, for every folder, and never asks again', async () => {
    const { git, asked } = world(() => Promise.reject({ code: 'unsupported', message: GIT_UNAVAILABLE }));

    git.want(['/repo', '/other'], 0);
    await settled();
    expect(asked).toEqual(['/repo']);

    // The one answer is for the whole app: the other folder, a reload and the time that passes ask nothing.
    expect(git.cell(place('a.txt'))).toEqual({ text: '–', tip: 'Git status is not available on Windows yet' });
    expect(git.cell(place('b.txt', '/other'))).toEqual({ text: '–', tip: GIT_UNAVAILABLE });
    git.want(['/repo', '/other', '/third'], 1);
    await vi.advanceTimersByTimeAsync(GIT_TTL_MS * 10);
    git.want(['/repo', '/other'], 2);
    await vi.advanceTimersByTimeAsync(GIT_TTL_MS * 10);

    expect(asked).toEqual(['/repo']);
  });

  it('writes out the characters of a reason that could pass for others', async () => {
    const { git } = world(() => Promise.reject({ code: 'invalidInput', message: 'bad‮name' }));

    git.want(['/repo'], 0);
    await settled();

    expect(git.cell(place('a.txt')).tip).not.toContain('‮');
  });

  it('keeps nothing when the request fails for another reason: the folder waits about 10 seconds and is asked again', async () => {
    let fail = true;
    const { git, asked } = world(() => (fail ? Promise.reject(refusal('internal')) : repo(['a.txt', 'modified'])));

    git.want(['/repo'], 0);
    await settled();
    expect(git.cell(place('a.txt'))).toEqual({ text: '' });

    fail = false;
    await vi.advanceTimersByTimeAsync(9_999);
    expect(asked).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);

    expect(asked).toHaveLength(2);
    expect(git.cell(place('a.txt')).text).toBe('M');
  });

  it('takes a failure of another kind than invalidInput, io and unsupported (not found, internal) for a passing one, not for a repository too big', async () => {
    const { git, asked } = world(() => Promise.reject(refusal('notFound')));

    git.want(['/repo'], 0);
    await settled();
    expect(git.cell(place('a.txt'))).toEqual({ text: '' });

    await vi.advanceTimersByTimeAsync(10_000);
    expect(asked).toHaveLength(2);
  });

  it('takes an answer of another shape for a failure, not for a folder outside a repository', async () => {
    const { git, asked } = world(() => 'oops');

    git.want(['/repo'], 0);
    await settled();
    git.want(['/repo'], 0);
    await settled();

    expect(asked).toHaveLength(1);
    expect(git.cell(place('a.txt'))).toEqual({ text: '' });
  });

  it('asks again for an answer that came after a reload, since it is out of date already', async () => {
    const answers = held();
    const { git, asked } = world(answers.answer);

    git.want(['/repo'], 0);
    await settled();
    git.want(['/repo'], 1);
    await settled();
    expect(asked).toHaveLength(1); // the first request is still out

    answers.out.get('/repo')?.resolve(repo(['a.txt', 'modified']));
    await settled();

    expect(asked).toHaveLength(2);
  });

  it('wants 100 folders at the most', async () => {
    const { git, asked } = world(() => OUTSIDE, { parallel: 1000 });

    git.want(Array.from({ length: 150 }, (_, i) => `/f${i}`), 0);
    await settled();

    expect(asked).toHaveLength(100);
  });

  it('asks again, when the time is up, only for folders still wanted, and 100 at the most', async () => {
    const { git, asked } = world(() => OUTSIDE, { parallel: 1000 });
    const folders = Array.from({ length: 150 }, (_, i) => `/f${i}`);

    git.want(folders, 0);
    await settled();
    expect(asked).toHaveLength(100);

    asked.length = 0;
    await vi.advanceTimersByTimeAsync(GIT_TTL_MS);
    expect(asked.sort()).toEqual(folders.slice(0, 100).sort());

    git.want(folders.slice(80, 150), 0); // /f80 to /f149: 70 wanted, of which /f80 to /f99 are kept still
    await settled();
    asked.length = 0;
    await vi.advanceTimersByTimeAsync(GIT_TTL_MS);

    expect(asked.sort()).toEqual(folders.slice(80, 150).sort());
  });

  it('sorts by how serious the mark is, an item with none (and one not known) having nothing to sort by', async () => {
    const { git } = world(() => repo(['c', 'conflicted'], ['m', 'modified'], ['s', 'staged'], ['u', 'untracked'], ['i', 'ignored']));

    expect(git.severity(place('m'))).toBeUndefined();
    git.want(['/repo'], 0);
    await settled();

    expect(['c', 'm', 's', 'u', 'i'].map((name) => git.severity(place(name)))).toEqual([0, 1, 2, 3, 4]);
    expect(git.severity(place('clean'))).toBeUndefined();
    expect(git.severity(undefined)).toBeUndefined();
  });
});
