// A test helper that shows a pane, whatever its layout.
import { render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { vi } from 'vitest';

import { Hotlist } from '$lib/panes/hotlist.svelte';
import type { PaneTabs } from '$lib/panes/tabs.svelte';
import { Volumes } from '$lib/panes/volumes.svelte';

import Pane from '../src/lib/shell/Pane.svelte';
import type { fakeBackend } from './panes';

/** Shows `tabs` in a pane, as it is (its layout is the tab's). */
export async function mountPane(backend: ReturnType<typeof fakeBackend>, tabs: PaneTabs) {
  render(Pane, {
    label: 'Left pane',
    tabs,
    volumes: new Volumes(backend.ipc),
    hotlist: new Hotlist(),
    menu: undefined,
    active: true,
    onActivate: vi.fn(),
    onMenu: vi.fn(),
  });
  await tick();
}
