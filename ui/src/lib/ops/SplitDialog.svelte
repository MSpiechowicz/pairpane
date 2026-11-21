<script lang="ts">
  import { untrack } from 'svelte';

  import type { Entry } from '$lib/ipc/bindings/Entry';
  import type { Transport } from '$lib/ipc/transport';
  import { formatSize } from '$lib/panes/format';
  
  import { what } from './dialogs';
  import JobDialog from './JobDialog.svelte';
  import { customPart, MAX_PARTS, MIN_PART, PART_PRESETS, PART_UNITS, partCount } from './jobs';
  import { Submission } from './submission.svelte';
  import './fields.css';

  let {
    ipc,
    path,
    entry,
    to,
    onClose,
    onDone,
  }: {
    ipc: Transport;
    /** The folder the file is in. */
    path: string;
    entry: Entry;
    /** The folder the parts go into to begin with. */
    to: string;
    onClose: () => void;
    /** The job started. */
    onDone: () => void;
  } = $props();

  /** What the size list holds: the bytes of a preset, or `custom`. */
  const CUSTOM = 'custom';

  let target = $state(untrack(() => to));
  let choice = $state<string>(String(PART_PRESETS[1].bytes));
  let amount = $state<number | null>(100);
  let unit = $state<number>(PART_UNITS[1].bytes);
  const submission = new Submission(() => onClose());

  const part = $derived(choice === CUSTOM ? customPart(amount ?? 0, unit) : Number(choice));
  const parts = $derived(part === undefined ? undefined : partCount(entry.size, part));

  /** What the dialog says about the size chosen. */
  const note = $derived.by(() => {
    if (entry.size === 0) {
      return `${what([entry.name])} is empty: there is nothing to split.`;
    }

    if (parts === undefined) {
      return `Parts must be at least ${MIN_PART / 1024} KB.`;
    }

    if (parts > MAX_PARTS) {
      return `That would make ${parts} parts; the most is ${MAX_PARTS}. Choose bigger parts.`;
    }

    return `${what([entry.name])} is ${formatSize(entry.size)}: ${parts === 1 ? '1 part' : `${parts} parts`}.`;
  });

  async function submit() {
    if (submission.busy || target.trim() === '') {
      return;
    }

    if (entry.size === 0 || part === undefined || parts === undefined || parts > MAX_PARTS) {
      submission.refuse(note);
      return;
    }

    await submission.run(async () => {
      await ipc.invoke('ops_task', { from: path, names: [entry.name], task: { kind: 'split', to: target, partSize: part } });
      onDone();
    });
  }
</script>

<JobDialog title="Split {what([entry.name])}" confirm="Split" {submission} onSubmit={submit}>
  <input
    class="field"
    type="text"
    spellcheck="false"
    autocomplete="off"
    aria-label="Target folder"
    data-autofocus
    bind:value={target}
  />
  <div class="size">
    <select class="field" aria-label="Part size" value={choice} onchange={(event) => (choice = event.currentTarget.value)}>
      {#each PART_PRESETS as { label, bytes } (bytes)}
        <option value={String(bytes)}>{label}</option>
      {/each}
      <option value={CUSTOM}>Custom size</option>
    </select>
    {#if choice === CUSTOM}
      <input class="field amount" type="number" min="1" step="any" aria-label="Custom size" bind:value={amount} />
      <select class="field" aria-label="Unit" value={unit} onchange={(event) => (unit = Number(event.currentTarget.value))}>
        {#each PART_UNITS as { label, bytes } (bytes)}
          <option value={bytes}>{label}</option>
        {/each}
      </select>
    {/if}
  </div>
  <p class="hint">{note}</p>
</JobDialog>

<style>
  .size {
    display: flex;
    gap: 8px;
  }

  .size > select:first-child {
    flex: 1;
    min-width: 0;
  }

  .amount {
    width: 110px;
  }
</style>
