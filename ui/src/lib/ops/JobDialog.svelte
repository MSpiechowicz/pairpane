<script lang="ts">
  import type { Snippet } from 'svelte';

  import Modal from '$lib/shell/Modal.svelte';

  import DialogFooter from './DialogFooter.svelte';
  import { enterSubmits, type Submission } from './submission.svelte';
  import './fields.css';

  let {
    title,
    confirm,
    submission,
    onSubmit,
    children,
  }: {
    title: string;
    /** The label of the button that starts the job. */
    confirm: string;
    submission: Submission;
    /** The button, or Enter in a field or a box. */
    onSubmit: () => void;
    children: Snippet;
  } = $props();
</script>

<Modal {title} onClose={submission.close}>
  <div class="body" {@attach enterSubmits(onSubmit)}>
    {@render children()}
  </div>
  <DialogFooter failure={submission.failure} busy={submission.busy} {confirm} onConfirm={onSubmit} onCancel={submission.close} />
</Modal>

<style>
  .body {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  /* The gap spaces the lines out already. */
  .body :global(.hint) {
    margin: 0;
  }
</style>
