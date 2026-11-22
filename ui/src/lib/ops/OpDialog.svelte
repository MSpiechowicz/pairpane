<script lang="ts">
  import { untrack } from 'svelte';

  import type { Transport } from '$lib/ipc/transport';
  import Modal from '$lib/shell/Modal.svelte';

  import { type Notice, perform, specOf } from './dialogs';
  import DialogFooter from './DialogFooter.svelte';
  import { Submission } from './submission.svelte';
  import type { OpRequest } from './targets';
  import './fields.css';

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
  const submission = new Submission(() => onClose());

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
    if (submission.busy || value.trim() === '') {
      return;
    }

    // The same name asks for nothing: the backend would refuse it as taken.
    if (request.kind === 'rename' && value === request.name) {
      onDone(value);
      return;
    }

    await submission.run(async () => {
      const done = await perform(ipc, request, value, verify);
      if (done.notice) {
        onNotice?.(done.notice);
      }

      onDone(done.name);
    });
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.isComposing && event.target instanceof HTMLInputElement) {
      event.preventDefault();
      void submit();
    }
  }
</script>

<Modal title={spec.title} onClose={submission.close}>
  <div class="row">
    <input
      class="field"
      type="text"
      spellcheck="false"
      autocomplete="off"
      aria-label={spec.label}
      readonly={spec.fixed}
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
  <DialogFooter failure={submission.failure} busy={submission.busy} confirm={spec.confirm} onConfirm={submit} onCancel={submission.close} />
</Modal>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .row > .field {
    flex: 1;
    min-width: 0;
  }
</style>
