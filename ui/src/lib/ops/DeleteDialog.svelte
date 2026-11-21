<script lang="ts">
  import Modal from '$lib/shell/Modal.svelte';

  import { trashName, what } from './dialogs';
  import DialogFooter from './DialogFooter.svelte';
  import { Submission } from './submission.svelte';

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

  const submission = new Submission(() => onClose());

  const confirm = () => submission.run(onConfirm);
</script>

<Modal {title} onClose={submission.close}>
  {#if permanent}
    <p class="note">This cannot be undone.</p>
  {/if}
  <DialogFooter
    failure={submission.failure}
    busy={submission.busy}
    confirm={permanent ? 'Delete' : `Move to ${trashName()}`}
    danger={permanent}
    focus={permanent ? 'cancel' : 'confirm'}
    onConfirm={confirm}
    onCancel={submission.close}
  />
</Modal>

<style>
  .note {
    margin: 0;
  }
</style>
