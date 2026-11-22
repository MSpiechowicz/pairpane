<script lang="ts">
  import { displayName } from '$lib/panes/format';
  import Modal from '$lib/shell/Modal.svelte';

  import DialogFooter from './DialogFooter.svelte';
  import type { Passwords, Prompt } from './passwords.svelte';
  import './fields.css';

  let { passwords, prompt }: { passwords: Passwords; prompt: Prompt } = $props();

  // What is typed lives only here, and only until it is sent.
  let value = $state('');

  const message = $derived(prompt.reason === 'wrong' ? 'The password is wrong. Try again.' : 'This archive needs a password.');

  async function submit() {
    if (prompt.busy || value === '') {
      return;
    }

    const typed = value;
    value = '';
    await passwords.submit(typed);
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.isComposing) {
      event.preventDefault();
      void submit();
    }
  }
</script>

<Modal title="Password for “{displayName(prompt.name)}”" onClose={() => passwords.cancel()}>
  <p class="message" role={prompt.reason === 'wrong' ? 'alert' : undefined}>{message}</p>
  <input
    class="field"
    type="password"
    spellcheck="false"
    autocomplete="off"
    aria-label="Password"
    data-autofocus
    bind:value
    {onkeydown}
  />
  <DialogFooter failure={prompt.failure} busy={prompt.busy} confirm="OK" onConfirm={submit} onCancel={() => passwords.cancel()} />
</Modal>

<style>
  .message {
    margin: 0 0 8px;
  }

  .field {
    width: 100%;
  }
</style>
