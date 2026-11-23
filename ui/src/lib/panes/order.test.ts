import { describe, expect, it } from 'vitest';

import type { Entry } from '$lib/ipc/bindings/Entry';

import { viewOrder, type ViewOptions } from './order';
import { entry } from '../../../tests/panes';

const options = (extra: Partial<ViewOptions> = {}): ViewOptions => ({
  column: 'name',
  desc: false,
  filter: '',
  sizeOf: () => undefined,
  ...extra,
});

const names = (entries: readonly (Entry | undefined)[], order: number[] | null) =>
  (order ?? entries.map((_, i) => i)).map((i) => entries[i]?.name);

const sample = [
  entry('docs', 'dir'),
  entry('src', 'dir'),
  entry('a.txt', 'file', { size: 30, modified: 300 }),
  entry('b.md', 'file', { size: 10, modified: 100 }),
  entry('c.txt', 'file', { size: 20, modified: 200 }),
  entry('d', 'file', { size: 10, modified: null }),
];

describe('viewOrder', () => {
  it('is the identity for name ascending without a filter', () => {
    expect(viewOrder(sample, options())).toBeNull();
    expect(viewOrder([], options())).toBeNull();
  });

  it('reverses folders and files separately for name descending', () => {
    expect(names(sample, viewOrder(sample, options({ desc: true })))).toEqual(['src', 'docs', 'd', 'c.txt', 'b.md', 'a.txt']);
  });

  it('sorts by extension with backend order breaking ties, in both directions', () => {
    expect(names(sample, viewOrder(sample, options({ column: 'ext' })))).toEqual(['docs', 'src', 'd', 'b.md', 'a.txt', 'c.txt']);
    expect(names(sample, viewOrder(sample, options({ column: 'ext', desc: true })))).toEqual(['docs', 'src', 'a.txt', 'c.txt', 'b.md', 'd']);
  });

  it('sorts by size with folders on top and ties in backend order', () => {
    expect(names(sample, viewOrder(sample, options({ column: 'size' })))).toEqual(['docs', 'src', 'b.md', 'd', 'c.txt', 'a.txt']);
    expect(names(sample, viewOrder(sample, options({ column: 'size', desc: true })))).toEqual(['docs', 'src', 'a.txt', 'c.txt', 'b.md', 'd']);
  });

  it('sorts folders by their computed size, unknown counting as zero', () => {
    const sizes: Record<string, number> = { src: 500 };
    const sizeOf = (e: Entry) => sizes[e.name];

    expect(names(sample, viewOrder(sample, options({ column: 'size', sizeOf })))).toEqual(['docs', 'src', 'b.md', 'd', 'c.txt', 'a.txt']);
    expect(names(sample, viewOrder(sample, options({ column: 'size', desc: true, sizeOf })))).toEqual(['src', 'docs', 'a.txt', 'c.txt', 'b.md', 'd']);
  });

  it('sorts by modified time, unknown as oldest', () => {
    expect(names(sample, viewOrder(sample, options({ column: 'modified' })))).toEqual(['docs', 'src', 'd', 'b.md', 'c.txt', 'a.txt']);
    expect(names(sample, viewOrder(sample, options({ column: 'modified', desc: true })))).toEqual(['docs', 'src', 'a.txt', 'c.txt', 'b.md', 'd']);
  });

  it('filters by a case-insensitive substring of the name', () => {
    expect(names(sample, viewOrder(sample, options({ filter: 'TXT' })))).toEqual(['a.txt', 'c.txt']);
    expect(names(sample, viewOrder(sample, options({ filter: 's', desc: true })))).toEqual(['src', 'docs']);
    expect(viewOrder(sample, options({ filter: 'zzz' }))).toEqual([]);
  });

  it('puts entries that are not loaded last and hides them from a filter', () => {
    const partial = [undefined, entry('b', 'file', { size: 2 }), undefined, entry('a', 'file', { size: 1 })];

    expect(viewOrder(partial, options({ column: 'size' }))).toEqual([3, 1, 0, 2]);
    expect(viewOrder(partial, options({ column: 'size', desc: true }))).toEqual([1, 3, 0, 2]);
    expect(viewOrder(partial, options({ filter: 'a' }))).toEqual([3]);
  });

  it('orders 100 000 entries quickly', () => {
    const many = Array.from({ length: 100_000 }, (_, i) =>
      entry(`file-${i}.${i % 7}`, i % 50 === 0 ? 'dir' : 'file', { size: (i * 7919) % 1000, modified: i }),
    );

    const started = performance.now();
    for (const column of ['ext', 'size', 'modified'] as const) {
      expect(viewOrder(many, options({ column, desc: true }))).toHaveLength(100_000);
    }
    expect(viewOrder(many, options({ filter: 'file-9' }))?.length).toBeGreaterThan(0);

    expect(performance.now() - started).toBeLessThan(3000);
  });
});

describe('viewOrder by an extra column', () => {
  const pics = [
    entry('docs', 'dir'),
    entry('a.jpg'),
    entry('b.jpg'),
    entry('c.jpg'),
    entry('d.jpg'),
    entry('e.jpg'),
  ];
  const known: Record<string, number | string> = { 'b.jpg': 300, 'd.jpg': 100, 'e.jpg': 300 };
  const extraOf = (e: Entry) => known[e.name];

  it('sorts by the value, ties in backend order, and puts what is not known last in either direction', () => {
    expect(names(pics, viewOrder(pics, options({ column: 'dimensions', extraOf })))).toEqual(['docs', 'd.jpg', 'b.jpg', 'e.jpg', 'a.jpg', 'c.jpg']);
    expect(names(pics, viewOrder(pics, options({ column: 'dimensions', desc: true, extraOf })))).toEqual([
      'docs',
      'b.jpg',
      'e.jpg',
      'd.jpg',
      'a.jpg',
      'c.jpg',
    ]);
  });

  it('sorts text (a checksum) as text', () => {
    const sums: Record<string, string> = { 'a.jpg': 'ff00', 'c.jpg': '0a0b', 'd.jpg': 'a1' };

    const order = viewOrder(pics, options({ column: 'checksum', extraOf: (e) => sums[e.name] }));

    expect(names(pics, order)).toEqual(['docs', 'c.jpg', 'd.jpg', 'a.jpg', 'b.jpg', 'e.jpg']);
  });

  it('keeps the backend order when nothing is known yet, and without a way to know', () => {
    const backend = ['docs', 'a.jpg', 'b.jpg', 'c.jpg', 'd.jpg', 'e.jpg'];

    expect(names(pics, viewOrder(pics, options({ column: 'dimensions', extraOf: () => undefined })))).toEqual(backend);
    expect(names(pics, viewOrder(pics, options({ column: 'checksum', desc: true })))).toEqual(backend);
  });

  it('applies the filter, and the unloaded entries stay last', () => {
    const entries = [...pics, undefined];

    expect(names(entries, viewOrder(entries, options({ column: 'dimensions', filter: '.jpg', extraOf })))).toEqual([
      'd.jpg',
      'b.jpg',
      'e.jpg',
      'a.jpg',
      'c.jpg',
    ]);
    expect(names(entries, viewOrder(entries, options({ column: 'dimensions', extraOf })))).toEqual([
      'docs',
      'd.jpg',
      'b.jpg',
      'e.jpg',
      'a.jpg',
      'c.jpg',
      undefined,
    ]);
  });
});
