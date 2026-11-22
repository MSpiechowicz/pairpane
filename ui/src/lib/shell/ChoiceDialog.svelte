<script lang="ts">
  import DialogFooter from '$lib/ops/DialogFooter.svelte';
  import { displayName } from '$lib/panes/format';

  import Modal from './Modal.svelte';

  let {
    title,
    message,
    confirm,
    alternate,
    danger = false,
    onConfirm,
    onAlternate,
    onCancel,
  }: {
    title: string;
    message?: string;
    /** The first choice (`Save`, `Overwrite`); the focus starts on it, or on Cancel when it cannot be taken back. */
    confirm: string;
    /** The second choice (`Don't save`, `Reload`), when there is one. */
    alternate?: string;
    danger?: boolean;
    onConfirm: () => void;
    onAlternate?: () => void;
    /** Cancel, and Escape. */
    onCancel: () => void;
  } = $props();
</script>

<Modal {title} onClose={onCancel}>
  {#if message}
    <p class="message">{displayName(message)}</p>
  {/if}
  <DialogFooter busy={false} {confirm} {alternate} {danger} focus={danger ? 'cancel' : 'confirm'} {onConfirm} {onAlternate} {onCancel} />
</Modal>

<style>
  .message {
    margin: 0;
  }
</style>
