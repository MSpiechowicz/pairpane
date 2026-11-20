// Helpers for tests of a pane's tabs: a pane that has listed `/a`, and moves the test lets arrive by hand.
import { PaneTabs } from '$lib/panes/tabs.svelte';

import { arrive, deliver, fakeBackend } from './panes';

/** Tabs whose pane has listed `/a`. */
export async function inA() {
  const backend = fakeBackend();
  const tabs = new PaneTabs(backend.ipc, 'left');
  await deliver(backend, () => tabs.view.listing.open('/a'), '/a', []);
  return { backend, tabs };
}

export type Backend = ReturnType<typeof fakeBackend>;

/** Navigates the way a click does and lets the folder arrive. */
export const go = (backend: Backend, tabs: PaneTabs, path: string) =>
  deliver(backend, () => tabs.view.listing.open(path), path, []);

/** The folder the last open asked for. */
export const asked = (backend: Backend) => backend.last().args.path;

/** Lets the folder the last open asked for arrive (the move of a back, forward or tab switch). */
export const arrived = (backend: Backend) => arrive(backend.last(), asked(backend) as string);

export const shown = (tabs: PaneTabs) => tabs.view.listing.path;

