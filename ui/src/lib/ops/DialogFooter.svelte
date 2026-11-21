<script lang="ts">
  let {
    failure,
    busy,
    confirm,
    cancel = 'Cancel',
    danger = false,
    focus,
    onConfirm,
    onCancel,
  }: {
    /** Why the last try was refused; shown above the buttons. */
    failure?: string;
    /** Something is being done: the buttons are off. */
    busy: boolean;
    /** The label of the button that does it; a dialog that only shows something has none. */
    confirm?: string;
    /** The label of the other button (`Close` for a dialog with nothing to confirm). */
    cancel?: string;
    /** The action cannot be taken back. */
    danger?: boolean;
    /** The button that takes the focus when the dialog opens; none when a field does. */
    focus?: 'confirm' | 'cancel';
    onConfirm?: () => void;
    onCancel: () => void;
  } = $props();
</script>

{#if failure}
  <p class="failure" role="alert">{failure}</p>
{/if}
<div class="mt-3 flex justify-end gap-2">
  <button class="btn" type="button" disabled={busy} data-autofocus={focus === 'cancel' ? '' : undefined} onclick={onCancel}>{cancel}</button>
  {#if confirm}
    <button
      class="btn {danger ? 'btn-danger' : 'btn-primary'}"
      type="button"
      disabled={busy}
      data-autofocus={focus === 'confirm' ? '' : undefined}
      onclick={onConfirm}>{confirm}</button
    >
  {/if}
</div>

<style>
  .failure {
    margin: 8px 0 0;
    color: var(--danger);
  }
</style>
