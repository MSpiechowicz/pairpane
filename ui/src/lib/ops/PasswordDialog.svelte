<script lang="ts">
  import Modal from '$lib/shell/Modal.svelte';

  import DialogFooter from './DialogFooter.svelte';
  import './fields.css';

  let {
    title,
    message,
    notice,
    alert = false,
    label = 'Password',
    failure,
    busy,
    alternate,
    onSubmit,
    onAlternate,
    onCancel,
  }: {
    title: string;
    /** What is asked, and why. */
    message: string;
    /** A line of its own above the message, told to assistive technology at once (why this is asked). */
    notice?: string;
    /** The message is a warning (the last try was wrong), told to assistive technology at once. */
    alert?: boolean;
    /** The accessible name of the field. */
    label?: string;
    /** Something other than a wrong secret went wrong when it was checked. */
    failure?: string;
    /** What was typed is being checked. */
    busy: boolean;
    /** The label of a button that asks for something else than what is asked (`Use password instead`). */
    alternate?: string;
    /** What was typed; it is not kept here once it is sent. */
    onSubmit: (value: string) => void | Promise<void>;
    /** The button of `alternate` was used; what was typed is dropped first. */
    onAlternate?: () => void;
    onCancel: () => void;
  } = $props();

  // What is typed lives only here, and only until it is sent.
  let value = $state('');
  let field = $state<HTMLInputElement>();

  // The field is off while a try is checked; it takes the focus back when the answer is in. Whatever got into it
  // meanwhile is dropped, so it is never sent as the answer to what the next question asks.
  let wasBusy = false;
  $effect(() => {
    if (busy) {
      wasBusy = true;
    } else if (wasBusy) {
      wasBusy = false;
      value = '';
      field?.focus();
    }
  });

  async function submit() {
    if (busy || value === '') {
      return;
    }

    const typed = value;
    value = '';
    await onSubmit(typed);
  }

  function switchKind() {
    value = '';
    onAlternate?.();
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.isComposing) {
      event.preventDefault();
      void submit();
    }
  }
</script>

<Modal {title} onClose={onCancel}>
  {#if notice}
    <p class="message" role="alert">{notice}</p>
  {/if}
  <p class="message" role={alert ? 'alert' : undefined}>{message}</p>
  <input
    class="field"
    type="password"
    spellcheck="false"
    autocomplete="off"
    aria-label={label}
    data-autofocus
    disabled={busy}
    bind:this={field}
    bind:value
    {onkeydown}
  />
  <DialogFooter {failure} {busy} confirm="OK" {alternate} onAlternate={switchKind} onConfirm={submit} {onCancel} />
</Modal>

<style>
  .message {
    margin: 0 0 8px;
  }

  .field {
    width: 100%;
  }
</style>
