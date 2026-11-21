<script lang="ts">
  import { onDestroy, untrack } from 'svelte';

  import Modal from '$lib/shell/Modal.svelte';

  import { ViewSearch } from './search.svelte';
  import type { ViewSession } from './session.svelte';
  import ViewBody from './ViewBody.svelte';
  import ViewerToolbar from './ViewerToolbar.svelte';

  let { session, onClose }: { session: ViewSession; onClose: () => void } = $props();

  const search = new ViewSearch(untrack(() => session));
  onDestroy(() => search.dispose());

  /** F3 and Shift+F3 find the next and previous match; Ctrl+F goes to the search box. */
  function onkeydown(event: KeyboardEvent) {
    if (event.isComposing || event.altKey || event.metaKey || !(session.shown === 'text' || session.shown === 'hex')) {
      return;
    }

    if (event.key === 'F3' && !event.ctrlKey) {
      event.preventDefault();
      search.run(event.shiftKey);
    } else if (event.key.toLowerCase() === 'f' && event.ctrlKey && !event.shiftKey) {
      event.preventDefault();
      document.querySelector<HTMLInputElement>('[data-search]')?.focus();
    }
  }
</script>

<svelte:window {onkeydown} />

<Modal title={session.name} variant="full" {onClose}>
  <div class="viewer">
    <ViewerToolbar {session} {search} {onClose} />
    <ViewBody {session} {search} focus />
  </div>
</Modal>

<style>
  .viewer {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
  }
</style>
