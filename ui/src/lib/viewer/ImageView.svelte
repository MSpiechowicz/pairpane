<script lang="ts">
  import { IpcFailure } from '$lib/ipc/transport';

  import { IMAGE_CAP, readWhole, TooBig } from './bytes';
  import type { PreviewProps } from './previews';
  import ViewNotice from './ViewNotice.svelte';

  let { session }: PreviewProps = $props();

  let url = $state<string | undefined>();
  let error = $state<string | undefined>();
  let loading = $state(true);

  // Reads the file and shows it as an image; the object URL is revoked when the view goes or the file changes.
  $effect(() => {
    const id = session.id;
    const kind = session.opened?.kind;
    const mime = kind?.kind === 'image' ? kind.mime : '';
    const controller = new AbortController();
    let made: string | undefined;

    url = undefined;
    error = undefined;
    loading = true;

    if (id !== undefined) {
      readWhole(session, IMAGE_CAP, controller.signal).then(
        (bytes) => {
          if (controller.signal.aborted) {
            return; // the view went (or the file changed) after the last chunk: nothing would revoke the URL
          }

          made = URL.createObjectURL(new Blob([bytes as BlobPart], { type: mime }));
          url = made;
          loading = false;
        },
        (reason: unknown) => {
          if (controller.signal.aborted) {
            return;
          }

          error = reason instanceof TooBig ? 'This image is too big to show here' : IpcFailure.from(reason).message;
          loading = false;
        },
      );
    }

    return () => {
      controller.abort();
      if (made !== undefined) {
        URL.revokeObjectURL(made);
      }
    };
  });
</script>

<div class="image">
  {#if url}
    <!-- An <img> only: a script in an SVG does not run there. -->
    <img
      src={url}
      alt={session.name}
      onerror={() => {
        url = undefined;
        error = 'This image cannot be shown here';
      }}
    />
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

  img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }
</style>
