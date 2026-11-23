import { describe, expect, it } from 'vitest';

import { MOST_SHOWN, Palette } from './model.svelte';

const palette = (folders: string[] = ['/home/me/docs', '/srv'], names: string[] = ['notes.txt', 'report.pdf']) =>
  new Palette({ folders: () => folders, names: () => names });

const labels = (shown: Palette) => shown.items.map((item) => item.label);

describe('Palette', () => {
  it('lists the commands, with their keys, and the latest folders while nothing is typed; never the names', () => {
    const shown = palette();
    shown.show();

    const copy = shown.items.find((item) => item.kind === 'command' && item.id === 'copy');
    expect(copy).toMatchObject({ label: 'Copy', keys: 'F5' });
    expect(shown.items.filter((item) => item.kind === 'folder').map((item) => item.label)).toEqual(['/home/me/docs', '/srv']);
    expect(shown.items.some((item) => item.kind === 'file')).toBe(false);
    expect(shown.items.length).toBeLessThanOrEqual(MOST_SHOWN);
  });

  it('finds the flat view by its name, with its keys', () => {
    const shown = palette();
    shown.show();

    shown.type('flat view');

    expect(shown.items[0]).toMatchObject({ kind: 'command', id: 'flatView', label: 'Flat view (all files in subfolders)', keys: 'Ctrl+B' });
  });

  it('finds a command, a folder or a name by what is typed, the best first, and chooses the first', () => {
    const shown = palette();
    shown.show();
    shown.move(3);

    shown.type('rep');
    expect(labels(shown)[0]).toBe('report.pdf');
    expect(shown.cursor).toBe(0);

    shown.type('docs');
    expect(shown.items[0]).toMatchObject({ kind: 'folder', path: '/home/me/docs' });

    shown.type('qzx');
    expect(shown.items).toEqual([]);
  });

  it('moves the choice round the ends and takes the chosen item, closing', () => {
    const shown = palette();
    shown.show();
    shown.type('new');
    const count = shown.items.length;
    expect(count).toBeGreaterThan(1);

    shown.move(-1);
    expect(shown.cursor).toBe(count - 1);
    shown.move(1);
    expect(shown.cursor).toBe(0);

    const taken = shown.take();
    expect(taken).toBeDefined();
    expect(shown.shown).toBe(false);
  });

  it('takes nothing when nothing matches, and stays open', () => {
    const shown = palette();
    shown.show();
    shown.type('qzx');

    expect(shown.take()).toBeUndefined();
    expect(shown.shown).toBe(true);
  });
});
