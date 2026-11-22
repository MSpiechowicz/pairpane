<script lang="ts">
  import { untrack } from 'svelte';

  import { displayName } from '$lib/panes/format';

  import type { Viewer } from './viewer.svelte';
  import ViewBody from './ViewBody.svelte';

  let { viewer }: { viewer: Viewer } = $props();

  const shown = $derived(viewer.quickShown);

  // Follows what the active cursor is on: at once the first time, then once the cursor rests.
  $effect(() => {
    const target = viewer.under;
    untrack(() => viewer.follow(target));

    return () => viewer.unfollow();
  });
</script>

<section class="quick" aria-label="Quick view">
  {#if shown === 'folder'}
    <p class="message">Folder</p>
  {:else if shown === undefined}
    <p class="message">Nothing to show</p>
  {:else}
    {#key shown}
      <header class="name">{displayName(shown.name)}</header>
      <ViewBody session={shown} />
    {/key}
  {/if}
</section>

<style>
  .quick {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--panel);
  }

  .name {
    padding: 8px 12px;
    border-bottom: 1px solid var(--line);
    overflow: hidden;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .message {
    margin: 0;
    padding: 28px 16px;
    color: var(--dim);
    text-align: center;
  }
</style>
