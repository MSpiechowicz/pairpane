<script lang="ts">
  import { displayName } from '$lib/panes/format';

  import { openPdf, type PdfDocument } from './pdf';
  import type { PreviewProps } from './previews';
  import { takeFocus } from './viewProps.svelte';
  import ViewNotice from './ViewNotice.svelte';

  let { session, focus = false }: PreviewProps = $props();

  const FAILED = 'This PDF cannot be shown here';
  const MARGIN = 12;

  let doc = $state.raw<PdfDocument | undefined>();
  let page = $state(1);
  let error = $state<string | undefined>();
  let canvas = $state<HTMLCanvasElement | undefined>();
  let width = $state(0);
  let height = $state(0);

  const pages = $derived(doc?.pages ?? 0);

  // Opens the document and frees it when the view goes.
  $effect(() => {
    if (session.id === undefined) {
      return;
    }

    const controller = new AbortController();
    let opened: PdfDocument | undefined;

    error = undefined;
    openPdf(session, controller.signal, () => (error = FAILED)).then(
      (document) => {
        opened = document;
        page = 1;
        doc = document;
      },
      () => {
        if (!controller.signal.aborted) {
          error = FAILED;
        }
      },
    );

    return () => {
      controller.abort();
      opened?.destroy();
      doc = undefined;
    };
  });

  // Draws the page again when it changes or the room for it does.
  $effect(() => {
    const current = doc;
    const target = canvas;
    const box = { width: Math.max(1, width - 2 * MARGIN), height: Math.max(1, height - 2 * MARGIN) };

    if (current !== undefined && target !== undefined && width > 0 && height > 0) {
      current.render(page, target, box).catch(() => (error = FAILED));
    }
  });

  function go(to: number) {
    page = Math.min(Math.max(1, to), Math.max(1, pages));
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'PageDown' || event.key === 'PageUp') {
      event.preventDefault();
      go(page + (event.key === 'PageDown' ? 1 : -1));
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions (PageUp and PageDown turn the page wherever the focus is inside) -->
<div class="pdf" role="document" aria-label="PDF {displayName(session.name)}" tabindex="-1" {onkeydown} {@attach takeFocus(focus)}>
  <div class="bar">
    <button type="button" disabled={page <= 1} aria-label="Previous page" onclick={() => go(page - 1)}>Prev</button>
    <span role="status">{pages > 0 ? `Page ${page} of ${pages}` : ''}</span>
    <button type="button" disabled={page >= pages} aria-label="Next page" onclick={() => go(page + 1)}>Next</button>
  </div>
  <div class="stage" bind:clientWidth={width} bind:clientHeight={height}>
    <canvas bind:this={canvas} aria-label="Page {page}" hidden={doc === undefined}></canvas>
  </div>
  <ViewNotice {error} text={doc === undefined && error === undefined ? 'Loading…' : undefined} />
</div>

<style>
  .pdf {
    position: relative;
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    outline: none;
  }

  .bar {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--line);
    color: var(--dim);
    font-size: 12px;
  }

  button {
    height: 26px;
    padding: 0 9px;
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--panel-2);
    color: var(--text);
    font: inherit;
  }

  button:disabled {
    opacity: 0.5;
  }

  button:not(:disabled):hover {
    border-color: color-mix(in srgb, var(--accent) 60%, var(--line));
    background: color-mix(in srgb, var(--accent) 16%, var(--panel-2));
  }

  .stage {
    display: flex;
    flex: 1;
    align-items: center;
    justify-content: center;
    min-height: 0;
    padding: 12px;
    overflow: auto;
  }

  canvas {
    background: white;
  }
</style>
