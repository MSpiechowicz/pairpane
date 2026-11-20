<script lang="ts">
  import { untrack } from 'svelte';

  import { IpcFailure, type Transport } from '$lib/ipc/transport';
  import Modal from '$lib/shell/Modal.svelte';

  import { type OpRequest, stemLength } from './targets';

  let {
    request,
    ipc,
    onClose,
    onDone,
  }: {
    request: OpRequest;
    ipc: Transport;
    onClose: () => void;
    /** The job started, or the item was renamed (to `name`). */
    onDone: (name: string) => void;
  } = $props();

  const rename = $derived(request.kind === 'rename');
  const verb = $derived({ copy: 'Copy', move: 'Move', rename: 'Rename' }[request.kind]);
  const title = $derived(request.kind === 'rename' ? verb : `${verb} ${what(request.names)} to`);

  // The dialog is made for one request: what the field starts with is read once.
  let value = $state(untrack(() => (request.kind === 'rename' ? request.name : request.to)));
  let verify = $state(false);
  let failure = $state<string | undefined>();
  let busy = $state(false);

  /** `“a.pdf”` for one item, `3 items` for more. */
  function what(names: string[]): string {
    return names.length === 1 ? `“${names[0]}”` : `${names.length} items`;
  }

  /** The field has the focus; a name has its stem selected so that typing keeps the extension. */
  function select(input: HTMLInputElement) {
    input.focus();
    if (request.kind === 'rename') {
      input.setSelectionRange(0, stemLength(request.name, request.dir));
    } else {
      input.select();
    }
  }

  async function run() {
    if (request.kind === 'rename') {
      await ipc.invoke('ops_rename', { path: request.path, from: request.name, to: value });
    } else {
      await ipc.invoke('ops_start', {
        kind: request.kind,
        from: request.from,
        names: request.names,
        to: value,
        verify,
      });
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
      await run();
      onDone(value);
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

<Modal {title} onClose={close}>
  <div class="row">
    <input
      type="text"
      spellcheck="false"
      autocomplete="off"
      aria-label={rename ? 'New name' : 'Target folder'}
      data-autofocus
      bind:value
      {onkeydown}
      {@attach select}
    />
    {#if !rename}
      <label class="check"><input type="checkbox" bind:checked={verify} {onkeydown} />Verify after {verb.toLowerCase()}</label>
    {/if}
  </div>
  {#if failure}
    <p class="failure" role="alert">{failure}</p>
  {/if}
  <div class="mt-3 flex justify-end gap-2">
    <button class="btn" type="button" disabled={busy} onclick={close}>Cancel</button>
    <button class="btn btn-primary" type="button" disabled={busy} onclick={submit}>{verb}</button>
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

  .failure {
    margin: 8px 0 0;
    color: var(--danger);
  }
</style>
