<script lang="ts">
  import { untrack } from 'svelte';

  import type { HashAlgorithm } from '$lib/ipc/bindings/HashAlgorithm';
  import type { Transport } from '$lib/ipc/transport';
  
  import { what } from './dialogs';
  import JobDialog from './JobDialog.svelte';
  import { checksumName, HASHES } from './jobs';
  import { Submission } from './submission.svelte';
  import './fields.css';

  let {
    ipc,
    path,
    names,
    onClose,
    onDone,
  }: {
    ipc: Transport;
    /** The folder the items are in; the checksum file is made there. */
    path: string;
    names: string[];
    onClose: () => void;
    /** The job started; `output` is the name of the file it makes. */
    onDone: (output: string) => void;
  } = $props();

  let algorithm = $state<HashAlgorithm>('sha256');
  /** The name the user typed; until then the name follows the algorithm. */
  let typed = $state<string | undefined>();
  const output = $derived(typed ?? checksumName(names, path, algorithm));
  const submission = new Submission(() => onClose());

  /** The field has the focus, with the name selected but not its extension. */
  function select(input: HTMLInputElement) {
    input.focus();
    input.setSelectionRange(0, untrack(() => output).lastIndexOf('.'));
  }

  async function submit() {
    if (submission.busy || output.trim() === '') {
      return;
    }

    await submission.run(async () => {
      await ipc.invoke('ops_task', { from: path, names, task: { kind: 'checksum', algorithm, output } });
      onDone(output);
    });
  }
</script>

<JobDialog title="Create checksum file for {what(names)}" confirm="Create" {submission} onSubmit={submit}>
  <fieldset class="algorithms">
    <legend class="sr-only">Checksum</legend>
    {#each HASHES as { algorithm: value, label } (value)}
      <label class="check"><input type="radio" name="algorithm" {value} bind:group={algorithm} />{label}</label>
    {/each}
  </fieldset>
  <input
    class="field"
    type="text"
    spellcheck="false"
    autocomplete="off"
    aria-label="Checksum file"
    data-autofocus
    value={output}
    oninput={(event) => (typed = event.currentTarget.value)}
    {@attach select}
  />
  <p class="hint">Made in this folder. Verify checksum file picks the checksum by the file's extension.</p>
</JobDialog>

<style>
  .algorithms {
    display: flex;
    gap: 16px;
    margin: 0;
    padding: 0;
    border: 0;
  }
</style>
