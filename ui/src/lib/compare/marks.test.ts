import { describe, expect, it } from 'vitest';

import { deliver, entry, fakeBackend } from '../../../tests/panes';
import { PaneView } from '../panes/view.svelte';
import type { Mark } from './quick';

async function shown() {
  const backend = fakeBackend();
  const view = new PaneView(backend.ipc, 'left');
  await deliver(backend, () => view.listing.open('/work'), '/work', [entry('a.txt'), entry('b.txt')]);
  view.marks.set(new Map<string, Mark>([['a.txt', 'onlyHere']]));
  return { backend, view };
}

describe('the marks of a pane', () => {
  it('are told by entry name', async () => {
    const { view } = await shown();

    expect(view.marks.get('a.txt')).toBe('onlyHere');
    expect(view.marks.get('b.txt')).toBeUndefined();
  });

  it('are gone when another folder starts listing', async () => {
    const { backend, view } = await shown();

    await deliver(backend, () => view.listing.open('/other'), '/other', [entry('a.txt')]);

    expect(view.marks.get('a.txt')).toBeUndefined();
  });

  it('are gone when the same folder is listed again', async () => {
    const { backend, view } = await shown();

    await deliver(backend, () => view.listing.reload(), '/work', [entry('a.txt'), entry('b.txt')]);

    expect(view.marks.get('a.txt')).toBeUndefined();
  });

  it('select exactly the names given, and drop the selection that was there', async () => {
    const { view } = await shown();
    view.selectAll();

    view.select(['b.txt']);

    expect(view.selectedShown().map(({ name }) => name)).toEqual(['b.txt']);
  });
});
