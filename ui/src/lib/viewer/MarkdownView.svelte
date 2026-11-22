<script lang="ts">
  import type { MdDocument } from '$lib/ipc/bindings/MdDocument';
  import { IpcFailure } from '$lib/ipc/transport';
  import { displayName } from '$lib/panes/format';

  import MdBlock from './MdBlock.svelte';
  import type { PreviewProps } from './previews';
  import { takeFocus } from './viewProps.svelte';
  import ViewNotice from './ViewNotice.svelte';

  let { session, focus = false }: PreviewProps = $props();

  let doc = $state.raw<MdDocument | undefined>();
  let error = $state<string | undefined>();

  // Reads the document, and again when the encoding changes; an answer to an earlier question is dropped.
  $effect(() => {
    const id = session.id;
    const encoding = session.encoding;
    let current = true;

    if (id !== undefined) {
      session.ipc.invoke('view_markdown', { session: id, encoding }).then(
        (answer) => {
          if (current) {
            doc = answer;
            error = undefined;
          }
        },
        (reason: unknown) => {
          if (current) {
            error = IpcFailure.from(reason).message;
          }
        },
      );
    }

    return () => {
      current = false;
    };
  });
</script>

<div class="markdown">
  <!-- svelte-ignore a11y_no_noninteractive_tabindex (a scrolling region takes the keyboard to be scrolled by it) -->
  <div class="page" role="document" aria-label="Rendered {displayName(session.name)}" tabindex="0" {@attach takeFocus(focus)}>
    {#if doc}
      <MdBlock blocks={doc.blocks} />
    {/if}
  </div>
  <ViewNotice {error} text={doc?.truncated ? 'Only the start of this file is shown' : undefined} />
</div>

<style>
  .markdown {
    position: relative;
    flex: 1;
    min-height: 0;
  }

  .page {
    height: 100%;
    padding: 12px 22px 40px 16px;
    overflow: auto;
    outline: none;
    line-height: 1.55;
    overflow-wrap: anywhere;
  }

  .page :global(h1),
  .page :global(h2),
  .page :global(h3),
  .page :global(h4),
  .page :global(h5),
  .page :global(h6) {
    margin: 16px 0 8px;
    line-height: 1.25;
  }

  .page :global(h1) {
    font-size: 1.6em;
  }

  .page :global(h2) {
    font-size: 1.35em;
  }

  .page :global(h3) {
    font-size: 1.15em;
  }
</style>
