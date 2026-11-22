import { describe, expect, it, vi } from 'vitest';

import { PaneTabs } from '$lib/panes/tabs.svelte';

import { compareBackend, folder } from '../../../tests/compare';
import { arrive, entry, fakeBackend } from '../../../tests/panes';
import { Comparison } from './controller.svelte';

/** Two panes, `/left` and `/right`, listed with the entries given. */
async function panes(left = [entry('a')], right = [entry('b')]) {
  const backend = fakeBackend();
  const tabs = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
  for (const [side, path, entries] of [
    ['left', '/left', left],
    ['right', '/right', right],
  ] as const) {
    await tabs[side].view.listing.open(path);
    arrive(backend.opens.at(-1)!, path, entries);
  }

  const say = vi.fn();
  const scans = compareBackend();
  const comparison = new Comparison(scans.ipc, tabs, say, () => false);
  return { backend, tabs, say, comparison, scans };
}

/** A pane showing the results of a search (the names are paths below the folder searched). */
async function showResults(tabs: PaneTabs, backend: ReturnType<typeof fakeBackend>) {
  await tabs.view.listing.openResults(1);
  const channel = backend.opens.at(-1)!;
  channel.emit({ kind: 'start', path: '/left', parent: '/', crumbs: [], total: 0, results: 1 });
  channel.emit({ kind: 'done', dirs: 0, files: 0, bytes: 0, free: null });
}

describe('Comparison', () => {
  it('refuses a pane that shows search results, for both ways of comparing', async () => {
    const { backend, tabs, say, comparison } = await panes();
    await showResults(tabs.right, backend);

    comparison.quick();
    comparison.synchronize();

    expect(say.mock.calls).toEqual([
      ['Open the file’s folder first.', true],
      ['Open the file’s folder first.', true],
    ]);
    expect(comparison.modal).toBe(false);
    expect(tabs.left.view.marks.get('a')).toBeUndefined();
  });

  it('refuses a pane whose folder could not be listed', async () => {
    const backend = fakeBackend();
    const tabs = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
    await tabs.left.view.listing.open('/left');
    arrive(backend.opens.at(-1)!, '/left', [entry('a')]);
    await tabs.right.view.listing.open('/gone');
    backend.opens.at(-1)!.emit({ kind: 'error', error: { code: 'notFound', message: 'No such folder' } });
    const say = vi.fn();

    new Comparison(compareBackend().ipc, tabs, say, () => false).quick();

    expect(say).toHaveBeenCalledWith('A folder could not be listed.', true);
  });

  it('opens the dialog on the folders of the panes, with hidden as the app has it', async () => {
    const { tabs, say, scans } = await panes();
    const comparison = new Comparison(scans.ipc, tabs, say, () => true);

    comparison.synchronize();
    await comparison.syncer.start();

    expect(comparison.modal).toBe(true);
    expect(scans.scans[0].args).toMatchObject({ left: folder('/left'), right: folder('/right'), options: { hidden: true } });
  });

  it('selects what a comparison picked and nothing else', async () => {
    const { tabs, comparison } = await panes([entry('a'), entry('b')], [entry('b'), entry('c')]);
    tabs.left.view.selectAll();

    comparison.quick();

    expect(tabs.left.view.selectedShown().map(({ name }) => name)).toEqual(['a']);
    expect(tabs.right.view.selectedShown().map(({ name }) => name)).toEqual(['c']);
  });
});
