<script lang="ts">
  import { untrack } from 'svelte';

  import type { AttributeChange } from '$lib/ipc/bindings/AttributeChange';
  import type { Entry } from '$lib/ipc/bindings/Entry';
  import type { Transport } from '$lib/ipc/transport';
  import { isWindowsPlatform } from '$lib/panes/keys';
  
  import { what } from './dialogs';
  import JobDialog from './JobDialog.svelte';
  import {
    agreed,
    formatStamp,
    MODE_COLUMNS,
    MODE_ROWS,
    MODE_SPECIAL,
    modeMasks,
    parseStamp,
    startModes,
    type Tri,
  } from './jobs';
  import { Submission } from './submission.svelte';
  import TriBox from './TriBox.svelte';
  import './fields.css';

  let {
    ipc,
    path,
    entries,
    onClose,
    onDone,
  }: {
    ipc: Transport;
    /** The folder the items are in. */
    path: string;
    entries: Entry[];
    onClose: () => void;
    /** The job started. */
    onDone: () => void;
  } = $props();

  // The dialog is made for one set of items: what the boxes start with is read once.
  const names = untrack(() => entries.map((entry) => entry.name));
  const folders = untrack(() => entries.some((entry) => entry.kind === 'dir'));
  const windows = isWindowsPlatform();

  const startMode = untrack(() => startModes(entries.map((entry) => entry.mode ?? 0)));
  const startReadOnly = untrack(() => agreed(entries.map((entry) => entry.readonly)));
  // The time the items have, when they have one in common; otherwise now.
  const startTime = untrack(() => formatStamp(entries.length === 1 ? (entries[0].modified ?? Date.now()) : Date.now()));

  let modes = $state<Record<number, Tri>>({ ...startMode });
  let readOnly = $state<Tri>(startReadOnly);
  let setModified = $state(false);
  let setAccessed = $state(false);
  let modified = $state(startTime);
  let accessed = $state(startTime);
  let recursive = $state(false);
  const submission = new Submission(() => onClose());

  /** The read-only attribute to set: sent when it was changed, or when what is inside folders gets it too. */
  function readOnlyChange(everything: boolean): boolean | null {
    if (readOnly === 'keep' || (!everything && readOnly === startReadOnly)) {
      return null;
    }

    return readOnly === 'on';
  }

  /** What the boxes ask for, or why it cannot be asked. */
  function change(): AttributeChange | string {
    const modifiedAt = setModified ? parseStamp(modified) : null;
    const accessedAt = setAccessed ? parseStamp(accessed) : null;
    if (modifiedAt === undefined || accessedAt === undefined) {
      return `Type a time like ${formatStamp(Date.now())}.`;
    }

    const everything = folders && recursive;
    const { add, remove } = windows ? { add: 0, remove: 0 } : modeMasks(startMode, modes, everything);
    const asked: AttributeChange = {
      mode: add === 0 && remove === 0 ? null : { kind: 'adjust', add, remove },
      readOnly: windows ? readOnlyChange(everything) : null,
      modified: modifiedAt,
      accessed: accessedAt,
      recursive: everything,
    };

    const nothing = asked.mode === null && asked.readOnly === null && modifiedAt === null && accessedAt === null;
    return nothing ? 'Choose what to change.' : asked;
  }

  async function submit() {
    if (submission.busy) {
      return;
    }

    const asked = change();
    if (typeof asked === 'string') {
      submission.refuse(asked);
      return;
    }

    await submission.run(async () => {
      await ipc.invoke('ops_task', { from: path, names, task: { kind: 'attributes', change: asked } });
      onDone();
    });
  }
</script>

<JobDialog title="Change attributes of {what(names)}" confirm="Apply" {submission} onSubmit={submit}>
  {#if windows}
    <label class="check"><TriBox bind:value={readOnly} mixed={startReadOnly === 'keep'} label="Read-only" />Read-only</label>
    <p class="hint">Only the read-only attribute of files can be changed.</p>
  {:else}
    <div class="modes" role="group" aria-label="Permissions">
      <span></span>
      {#each MODE_COLUMNS as column (column)}
        <span class="head">{column}</span>
      {/each}
      {#each MODE_ROWS as row (row.label)}
        <span class="head">{row.label}</span>
        {#each row.bits as bit, column (bit)}
          <TriBox bind:value={modes[bit]} mixed={startMode[bit] === 'keep'} label="{row.label} {MODE_COLUMNS[column].toLowerCase()}" />
        {/each}
      {/each}
    </div>
    <div class="special">
      {#each MODE_SPECIAL as { label, bit } (bit)}
        <label class="check"><TriBox bind:value={modes[bit]} mixed={startMode[bit] === 'keep'} {label} />{label}</label>
      {/each}
    </div>
    <p class="hint">A box that shows a dash leaves that permission as it is.</p>
  {/if}

  <div class="time">
    <label class="check"><input type="checkbox" bind:checked={setModified} />Modified</label>
    <input class="field" type="text" spellcheck="false" autocomplete="off" aria-label="Modified time" bind:value={modified} oninput={() => (setModified = true)} />
  </div>
  <div class="time">
    <label class="check"><input type="checkbox" bind:checked={setAccessed} />Accessed</label>
    <input class="field" type="text" spellcheck="false" autocomplete="off" aria-label="Accessed time" bind:value={accessed} oninput={() => (setAccessed = true)} />
  </div>

  {#if folders}
    <label class="check"><input type="checkbox" bind:checked={recursive} />Include what is inside folders</label>
  {/if}
</JobDialog>

<style>
  .modes {
    display: grid;
    grid-template-columns: repeat(4, max-content);
    gap: 6px 22px;
    align-items: center;
    justify-items: center;
  }

  .head {
    color: var(--dim);
  }

  .modes > .head:nth-child(n + 5) {
    justify-self: start;
  }

  .special {
    display: flex;
    gap: 16px;
  }

  .time {
    display: flex;
    align-items: center;
    gap: 12px;
    align-self: stretch;
  }

  .time .check {
    width: 90px;
  }

  .time .field {
    flex: 1;
    min-width: 0;
  }
</style>
