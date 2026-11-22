import { describe, expect, it, vi } from 'vitest';

import type { FindHit } from '$lib/ipc/bindings/FindHit';

import { goTo } from './goto';

const hit = (path: string, inner: string | null = null): FindHit => ({
  path,
  inner,
  kind: 'file',
  link: false,
  size: 1,
  modified: null,
  matches: null,
  line: null,
  lineNumber: null,
});

function pane() {
  const calls: string[] = [];
  const view = {
    listing: {
      open: vi.fn(async (path: string) => void calls.push(`open ${path}`)),
      openArchive: vi.fn(async (spot: { archive: string; dir: string }) => void calls.push(`archive ${spot.archive} ${spot.dir}`)),
    },
    pointAt: vi.fn((name: string) => void calls.push(`point ${name}`)),
  };
  return { view, calls };
}

describe('goTo', () => {
  it('opens the folder of the result and then puts the cursor on it', async () => {
    const { view, calls } = pane();

    await goTo(view, '/home/ann', hit('src/lib/a.rs'));

    expect(calls).toEqual(['open /home/ann/src/lib', 'point a.rs']);
  });

  it('a result in the folder searched opens that folder', async () => {
    const { view, calls } = pane();

    await goTo(view, '/home/ann/', hit('notes.txt'));

    expect(calls).toEqual(['open /home/ann/', 'point notes.txt']);
  });

  it('joins with the separator of the folder searched', async () => {
    const { view, calls } = pane();

    await goTo(view, 'C:\\Users\\ann', hit('docs/a.txt'));

    expect(calls).toEqual(['open C:\\Users\\ann\\docs', 'point a.txt']);
  });

  it('a result inside an archive opens the archive at the folder of the entry, with the cursor on the entry', async () => {
    const { view, calls } = pane();

    await goTo(view, '/data', hit('backups/old.zip', 'photos/2020/a.jpg'));

    expect(calls).toEqual(['archive /data/backups/old.zip photos/2020', 'point a.jpg']);
  });

  it('an entry at the root of an archive opens the root', async () => {
    const { view, calls } = pane();

    await goTo(view, '/data', hit('old.zip', 'a.jpg'));

    expect(calls).toEqual(['archive /data/old.zip ', 'point a.jpg']);
  });
});
