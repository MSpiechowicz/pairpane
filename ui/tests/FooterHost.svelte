<script lang="ts">
  // Test helper: a footer inside a modal, which is what gives the button marked `data-autofocus` the focus.
  import DialogFooter from '$lib/ops/DialogFooter.svelte';
  import Modal from '$lib/shell/Modal.svelte';

  let { danger, focus }: { danger: boolean; focus: 'confirm' | 'cancel' } = $props();
</script>

<Modal title="Delete">
  <DialogFooter busy={false} confirm="Delete" {danger} {focus} onConfirm={() => {}} onCancel={() => {}} />
</Modal>
