<script lang="ts">
  import { IpcFailure } from '$lib/ipc/transport';
  import Modal from '$lib/shell/Modal.svelte';

  import { trashName, what } from './dialogs';

  let {
    permanent,
    names,
    onConfirm,
    onClose,
  }: {
    /** Delete for good (danger, the focus on Cancel) instead of moving to the Trash. */
    permanent: boolean;
    names: string[];
    /** Does it; a refusal rejects and is shown here, with the dialog staying open. */
    onConfirm: () => Promise<void>;
    onClose: () => void;
  } = $props();

  const title = $derived(permanent ? `Delete ${what(names)} permanently?` : `Move ${what(names)} to the ${trashName()}?`);

  let failure = $state<string | undefined>();
  let busy = $state(false);

  async function confirm() {
    if (busy) {
      return;
    }

    busy = true;
    failure = undefined;
    try {
      await onConfirm();
    } catch (error) {
      failure = IpcFailure.from(error).message;
    } finally {
      busy = false;
    }
  }

  /** Escape and Cancel; what is being done cannot be called off from here. */
  function close() {
    if (!busy) {
      onClose();
    }
  }
</script>

<Modal {title} onClose={close}>
  {#if permanent}
    <p class="note">This cannot be undone.</p>
  {/if}
  {#if failure}
    <p class="failure" role="alert">{failure}</p>
  {/if}
  <div class="mt-3 flex justify-end gap-2">
    <button class="btn" type="button" disabled={busy} data-autofocus={permanent ? '' : undefined} onclick={close}>Cancel</button>
    <button
      class="btn {permanent ? 'btn-danger' : 'btn-primary'}"
      type="button"
      disabled={busy}
      data-autofocus={permanent ? undefined : ''}
      onclick={confirm}>{permanent ? 'Delete' : `Move to ${trashName()}`}</button
    >
  </div>
</Modal>

<style>
  .note,
  .failure {
    margin: 0;
  }

  .note + .failure {
    margin-top: 8px;
  }

  .failure {
    color: var(--danger);
  }
</style>
