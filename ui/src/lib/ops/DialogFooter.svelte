<script lang="ts">
  import { displayName } from '$lib/panes/format';

  let {
    failure,
    busy,
    disabled = false,
    confirm,
    cancel = 'Cancel',
    alternate,
    danger = false,
    focus,
    onConfirm,
    onAlternate,
    onCancel,
  }: {
    /** Why the last try was refused; shown above the buttons. */
    failure?: string;
    /** Something is being done: the buttons are off. */
    busy: boolean;
    /** The button that does it is off because the dialog is not ready (what it asks is not complete or not allowed). */
    disabled?: boolean;
    /** The label of the button that does it; a dialog that only shows something has none. */
    confirm?: string;
    /** The label of the other button (`Close` for a dialog with nothing to confirm). */
    cancel?: string;
    /** The label of a third button, between the two (`Don't save`). */
    alternate?: string;
    /** The action cannot be taken back. */
    danger?: boolean;
    /** The button that takes the focus when the dialog opens; none when a field does. */
    focus?: 'confirm' | 'cancel';
    onConfirm?: () => void;
    onAlternate?: () => void;
    onCancel: () => void;
  } = $props();
</script>

{#if failure}
  <p class="failure" role="alert">{displayName(failure)}</p>
{/if}
<div class="mt-3 flex justify-end gap-2">
  <button class="btn" type="button" disabled={busy} data-autofocus={focus === 'cancel' ? '' : undefined} onclick={onCancel}>{cancel}</button>
  {#if alternate}
    <button class="btn" type="button" disabled={busy} onclick={onAlternate}>{alternate}</button>
  {/if}
  {#if confirm}
    <button
      class="btn {danger ? 'btn-danger' : 'btn-primary'}"
      type="button"
      disabled={busy || disabled}
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
