import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, expect, it, vi } from 'vitest';

import { Hotlist } from '$lib/panes/hotlist.svelte';
import { PaneTabs } from '$lib/panes/tabs.svelte';

import { deliverRemote, fakeBackend } from '../../../tests/panes';
import HotlistMenu from './HotlistMenu.svelte';

describe('HotlistMenu on a server', () => {
  it('cannot add the server’s folder, and says why in words about servers', async () => {
    const backend = fakeBackend();
    const tabs = new PaneTabs(backend.ipc, 'left');
    await deliverRemote(backend, tabs.view.listing, '/srv');
    const hotlist = new Hotlist();
    render(HotlistMenu, { tabs, hotlist, onClose: vi.fn() });
    await tick();

    const add = screen.getByRole('button', { name: 'Add current folder' });
    await fireEvent.click(add);

    expect(add).toBeDisabled();
    expect(add).toHaveAttribute('title', 'A folder on a server can’t be a favourite.');
    expect(hotlist.has('/srv')).toBe(false);
  });
});
