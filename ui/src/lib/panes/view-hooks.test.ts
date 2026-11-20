import { describe, expect, it, vi } from 'vitest';

import { deliver, fakeBackend } from '../../../tests/panes';
import { PaneView } from './view.svelte';

describe('PaneView hooks', () => {
  it('onListed hears every folder that starts listing, a reload of the shown one included', async () => {
    const backend = fakeBackend();
    const onListed = vi.fn();
    const view = new PaneView(backend.ipc, 'left', { onListed });

    await deliver(backend, () => view.listing.open('/a'), '/a', []);
    await deliver(backend, () => view.listing.reload(), '/a', []);
    await deliver(backend, () => view.listing.open('/b'), '/b', []);

    expect(onListed.mock.calls).toEqual([['/a'], ['/a'], ['/b']]);
  });

  it('onOpen hears every open, a reload included, before its events', async () => {
    const backend = fakeBackend();
    const onOpen = vi.fn();
    const view = new PaneView(backend.ipc, 'left', { onOpen });

    await view.listing.open('/a');
    await view.listing.reload();

    expect(onOpen).toHaveBeenCalledTimes(2);
  });

  it('restoreSort sets the column and direction as given; restoring the same sort does not flip it', async () => {
    const backend = fakeBackend();
    const view = new PaneView(backend.ipc, 'left');
    await deliver(backend, () => view.listing.open('/a'), '/a', []);

    view.restoreSort('size', true);
    expect([view.column, view.desc]).toEqual(['size', true]);

    view.restoreSort('size', true); // restoring is not toggling
    expect([view.column, view.desc]).toEqual(['size', true]);

    view.restoreSort('name', false);
    expect([view.column, view.desc]).toEqual(['name', false]);
  });
});
