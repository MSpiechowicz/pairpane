<script lang="ts">
  import { IpcFailure } from '$lib/ipc/transport';
  import { displayName } from '$lib/panes/format';

  import { retryBusy } from './busy';
  import { IMAGE_CAP, readWhole, TooBig } from './bytes';
  import { type Decoded, drawDecoded, parseDecoded } from './decoded';
  import type { PreviewProps } from './previews';
  import ViewNotice from './ViewNotice.svelte';

  let { session }: PreviewProps = $props();

  // The backend decodes one picture at a time and a big one takes seconds, so a decode waits longer for its turn.
  const DECODE_BACKOFF_MS = [250, 500, 1000, 2000, 4000];

  let url = $state<string | undefined>();
  let error = $state<string | undefined>();
  let loading = $state(true);

  // A HEIC image the webview cannot show is decoded by the backend and drawn on a canvas.
  let decoded = $state.raw<Decoded | undefined>();
  let canvas = $state<HTMLCanvasElement>();

  // What the current file's read is tied to, for the decode that may follow it.
  let controller = new AbortController();
  let mime = '';
  let made: string | undefined;

  // Reads the file and shows it as an image; the object URL is revoked when the view goes or the file changes.
  $effect(() => {
    const id = session.id;
    const kind = session.opened?.kind;

    controller = new AbortController();
    mime = kind?.kind === 'image' ? kind.mime : '';
    made = undefined;

    const own = controller;

    url = undefined;
    decoded = undefined;
    error = undefined;
    loading = true;

    if (id !== undefined) {
      readWhole(session, IMAGE_CAP, own.signal).then(
        (bytes) => {
          if (own.signal.aborted) {
            return; // the view went (or the file changed) after the last chunk: nothing would revoke the URL
          }

          made = URL.createObjectURL(new Blob([bytes as BlobPart], { type: mime }));
          url = made;
          loading = false;
        },
        (reason: unknown) => {
          if (own.signal.aborted) {
            return;
          }

          error = reason instanceof TooBig ? 'This image is too big to show here' : IpcFailure.from(reason).message;
          loading = false;
        },
      );
    }

    return () => {
      own.abort();
      revoke();
    };
  });

  function revoke() {
    if (made !== undefined) {
      URL.revokeObjectURL(made);
      made = undefined;
    }
  }

  // The webview could not show the image: HEIC goes to the backend to decode, anything else is given up on.
  async function failed() {
    const id = session.id;
    const signal = controller.signal;

    url = undefined;
    revoke();

    if (mime !== 'image/heic' || id === undefined || signal.aborted) {
      error = 'This image cannot be shown here';
      return;
    }

    loading = true;

    let answer: ArrayBuffer;
    try {
      answer = await retryBusy(() => session.ipc.invoke('view_image_decode', { session: id }), signal, DECODE_BACKOFF_MS);
    } catch (reason: unknown) {
      if (!signal.aborted) {
        error = backendWords(IpcFailure.from(reason));
        loading = false;
      }

      return;
    }

    if (signal.aborted) {
      return;
    }

    try {
      decoded = parseDecoded(answer);
    } catch {
      error = 'This image cannot be shown here';
    }

    loading = false;
  }

  /** What to tell about a failed decode: the backend's hint when there is no decoder, else its words. */
  function backendWords(failure: IpcFailure): string {
    if (failure.code !== 'unsupported') {
      return failure.message;
    }

    const hint = failure.message.split(': ').slice(1).join(': ');

    // Without a hint (macOS, where the webview is the decoder) the webview simply could not show this file.
    return hint === '' ? 'This image cannot be shown here' : `No HEIC decoder on this system: ${hint}`;
  }

  // Draws the decoded pixels once the canvas is mounted.
  $effect(() => {
    if (canvas === undefined || decoded === undefined) {
      return;
    }

    const drawn = drawDecoded(canvas, decoded);
    if (drawn === 'no-context') {
      error = 'This image cannot be shown here';
    } else if (drawn === 'too-big') {
      // A very big picture can be refused by the webview (out of memory): say so instead of failing silently.
      decoded = undefined;
      error = 'This image is too big to show here';
    }
  });
</script>

<div class="image">
  {#if url}
    <!-- An <img> only: a script in an SVG does not run there. -->
    <img
      src={url}
      alt={displayName(session.name)}
      onerror={failed}
    />
  {/if}
  {#if decoded}
    <!-- svelte-ignore a11y_no_interactive_element_to_noninteractive_role -- a canvas is the picture itself, not a control -->
    <canvas bind:this={canvas} role="img" aria-label={displayName(session.name)} width={decoded.width} height={decoded.height}></canvas>
  {/if}
  <ViewNotice {error} text={loading ? 'Loading…' : undefined} />
</div>

<style>
  .image {
    position: relative;
    display: flex;
    flex: 1;
    align-items: center;
    justify-content: center;
    min-height: 0;
    padding: 12px;
  }

  img,
  canvas {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }
</style>
