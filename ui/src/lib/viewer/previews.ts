import type { Component } from 'svelte';

import type { ViewKind } from '$lib/ipc/bindings/ViewKind';

import ArchiveView from './ArchiveView.svelte';
import ImageView from './ImageView.svelte';
import MarkdownView from './MarkdownView.svelte';
import MediaView from './MediaView.svelte';
import PdfView from './PdfView.svelte';
import type { ViewSession } from './session.svelte';

/** What every preview gets: the session to draw (its `shown` is `preview`), and whether it takes the keyboard. */
export interface PreviewProps {
  session: ViewSession;
  focus?: boolean;
}

/**
 * The component that draws the preview of a kind (an image, a PDF page, audio or video, rendered Markdown, an
 * archive's entries); used by {@link ViewBody}. A kind without one shows a placeholder; Text and Hex stay available
 * for every kind, whatever is registered here.
 */
export const PREVIEWS: Partial<Record<ViewKind['kind'], Component<PreviewProps>>> = {
  markdown: MarkdownView,
  image: ImageView,
  pdf: PdfView,
  audio: MediaView,
  video: MediaView,
  archive: ArchiveView,
};
