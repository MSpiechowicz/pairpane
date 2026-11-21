<script lang="ts">
  import { convertFileSrc } from '@tauri-apps/api/core';

  import { IpcFailure } from '$lib/ipc/transport';

  import type { PreviewProps } from './previews';
  import ViewNotice from './ViewNotice.svelte';

  let { session }: PreviewProps = $props();

  const video = $derived(session.kind === 'video');

  let src = $state<string | undefined>();
  let error = $state<string | undefined>();
  let player = $state<HTMLMediaElement | undefined>();

  // Asks the backend for the address of the file; it is good until the session closes.
  $effect(() => {
    const id = session.id;
    let current = true;

    src = undefined;
    error = undefined;

    if (id !== undefined) {
      session.ipc.invoke('view_media', { session: id }).then(
        (token) => {
          if (current) {
            src = convertFileSrc(token, 'pairpane-media');
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

  // Lets go of the file when the view goes: the element stops reading it.
  $effect(() => {
    const element = player;

    return () => {
      if (element !== undefined) {
        element.removeAttribute('src');
        element.load();
      }
    };
  });

  const failed = () => (error = 'This file cannot be played here');
</script>

<div class="media">
  {#if src !== undefined}
    {#if video}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video bind:this={player} {src} controls preload="metadata" onerror={failed}></video>
    {:else}
      <audio bind:this={player} {src} controls preload="metadata" onerror={failed}></audio>
    {/if}
  {/if}
  <ViewNotice {error} />
</div>

<style>
  .media {
    position: relative;
    display: flex;
    flex: 1;
    align-items: center;
    justify-content: center;
    min-height: 0;
    padding: 12px;
  }

  video {
    max-width: 100%;
    max-height: 100%;
  }

  audio {
    width: min(100%, 520px);
  }
</style>
