import { describe, expect, it } from 'vitest';

import { Hotlist, MAX_HOTLIST } from './hotlist.svelte';

describe('Hotlist', () => {
  it('keeps the folders in the order they were added, each once', () => {
    const hotlist = new Hotlist();

    expect(hotlist.add('/b')).toBe(true);
    expect(hotlist.add('/a')).toBe(true);
    expect(hotlist.add('/b')).toBe(false);

    expect(hotlist.paths).toEqual(['/b', '/a']);
    expect(hotlist.has('/a')).toBe(true);
    expect(hotlist.has('/c')).toBe(false);
  });

  it('stops adding at the cap', () => {
    const hotlist = new Hotlist();
    for (let index = 0; index < MAX_HOTLIST; index += 1) {
      hotlist.add(`/folder${index}`);
    }

    expect(hotlist.add('/one-too-many')).toBe(false);
    expect(hotlist.paths).toHaveLength(MAX_HOTLIST);
  });

  it('removes a folder and leaves the others in place', () => {
    const hotlist = new Hotlist();
    hotlist.restore(['/a', '/b', '/c']);

    hotlist.remove('/b');
    hotlist.remove('/missing');

    expect(hotlist.paths).toEqual(['/a', '/c']);
  });

  it('filters by a case-insensitive part of the path', () => {
    const hotlist = new Hotlist();
    hotlist.restore(['/home/me/Projects', '/media/USB', '/home/me/Music']);

    expect(hotlist.filter('MUS')).toEqual(['/home/me/Music']);
    expect(hotlist.filter('/home')).toEqual(['/home/me/Projects', '/home/me/Music']);
    expect(hotlist.filter('')).toEqual(hotlist.paths);
    expect(hotlist.filter('nothing')).toEqual([]);
  });

  it('restores a saved list without duplicates and within the cap', () => {
    const hotlist = new Hotlist();
    hotlist.restore(['/a', '/a', ...Array.from({ length: MAX_HOTLIST + 5 }, (_, index) => `/f${index}`)]);

    expect(hotlist.paths[0]).toBe('/a');
    expect(hotlist.paths).toHaveLength(MAX_HOTLIST);
    expect(new Set(hotlist.paths).size).toBe(MAX_HOTLIST);
  });
});
