<script lang="ts">
  import { untrack } from 'svelte';

  import { IpcFailure, type Transport } from '$lib/ipc/transport';
  import Modal from '$lib/shell/Modal.svelte';

  import { type Notice, perform, specOf } from './dialogs';
  import type { OpRequest } from './targets';

  let {
    request,
    ipc,
    onClose,
    onDone,
    onNotice,
  }: {
    request: OpRequest;
    ipc: Transport;
    onClose: () => void;
    /** The job started, the item was renamed or made, or the link was made (its name is `name`). */
    onDone: (name: string) => void;
    /** Called before `onDone` when there is something to tell: a new file that was not opened. */
    onNotice?: (notice: Notice) => void;
  } = $props();

  const spec = $derived(specOf(request));

  // The dialog is made for one request: what the field starts with is read once.
  let value = $state(untrack(() => spec.initial));
  let verify = $state(false);
  let failure = $state<string | undefined>();
  let busy = $state(false);

  /** The field has the focus; a name has its stem selected so that typing keeps the extension. */
  function select(input: HTMLInputElement) {
    input.focus();
    if (spec.stem === undefined) {
      input.select();
    } else {
      input.setSelectionRange(0, spec.stem);
    }
  }

  async function submit() {
    if (busy || value.trim() === '') {
      return;
    }

    // The same name asks for nothing: the backend would refuse it as taken.
    if (request.kind === 'rename' && value === request.name) {
      onDone(value);
      return;
    }

    busy = true;
    failure = undefined;
    try {
      const done = await perform(ipc, request, value, verify);
      if (done.notice) {
        onNotice?.(done.notice);
      }

      onDone(done.name);
    } catch (error) {
      failure = IpcFailure.from(error).message;
    } finally {
      busy = false;
    }
  }

  /** Escape and Cancel; a job that is being started cannot be called off from here (its owner needs `onDone`). */
  function close() {
    if (!busy) {
      onClose();
    }
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.isComposing && event.target instanceof HTMLInputElement) {
      event.preventDefault();
      void submit();
    }
  }
</script>

<Modal title={spec.title} onClose={close}>
  <div class="row">
    <input
      type="text"
      spellcheck="false"
      autocomplete="off"
      aria-label={spec.label}
      data-autofocus
      bind:value
      {onkeydown}
      {@attach select}
    />
    {#if spec.verify}
      <label class="check"><input type="checkbox" bind:checked={verify} {onkeydown} />Verify after {spec.confirm.toLowerCase()}</label>
    {/if}
  </div>
  {#if spec.hint}
    <p class="hint">{spec.hint}</p>
  {/if}
  {#if failure}
    <p class="failure" role="alert">{failure}</p>
  {/if}
  <div class="mt-3 flex justify-end gap-2">
    <button class="btn" type="button" disabled={busy} onclick={close}>Cancel</button>
    <button class="btn btn-primary" type="button" disabled={busy} onclick={submit}>{spec.confirm}</button>
  </div>
</Modal>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .row > input[type='text'] {
    flex: 1;
    min-width: 0;
    height: 30px;
    padding: 0 10px;
    border: 1px solid var(--line);
    border-radius: 7px;
    background: var(--bg);
    color: var(--text);
    font: 12.5px var(--font-mono);
  }

  .row > input[type='text']:focus {
    border-color: var(--accent);
    outline: none;
  }

  .hint,
  .failure {
    margin: 8px 0 0;
  }

  .hint {
    color: var(--dim);
  }

  .failure {
    color: var(--danger);
  }
</style>
