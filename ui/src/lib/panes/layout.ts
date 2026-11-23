import type { PaneLayout } from '$lib/ipc/bindings/PaneLayout';

/** How a pane shows its folder: detailed, brief or thumbnails, as the session saves it. */
export type Layout = PaneLayout;

/** The layout as the saved state writes it (the same value: one the backend does not know reads as detailed). */
export function savedLayout(layout: Layout): PaneLayout {
  return layout;
}
