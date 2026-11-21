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
  // The Windows attributes. A pane entry knows read-only and hidden; the others are not known, so they start left alone.
  const ATTRIBUTES = ['readOnly', 'hidden', 'system', 'archive'] as const;
  const LABELS = { readOnly: 'Read-only', hidden: 'Hidden', system: 'System', archive: 'Archive' } as const;
  type Attribute = (typeof ATTRIBUTES)[number];
  const startAttrs = untrack<Record<Attribute, Tri>>(() => ({
    readOnly: agreed(entries.map((entry) => entry.readonly)),
    hidden: agreed(entries.map((entry) => entry.hidden)),
    system: 'keep',
    archive: 'keep',
  }));
  // The time the items have, when they have one in common; otherwise now.
  const startTime = untrack(() => formatStamp(entries.length === 1 ? (entries[0].modified ?? Date.now()) : Date.now()));

  let modes = $state<Record<number, Tri>>({ ...startMode });
  let attrs = $state<Record<Attribute, Tri>>({ ...startAttrs });
  let setModified = $state(false);
  let setAccessed = $state(false);
  let modified = $state(startTime);
  let accessed = $state(startTime);
  let recursive = $state(false);
  const submission = new Submission(() => onClose());

  /** The attributes to set: each is sent when it was changed, or when what is inside folders gets it too. */
  function attributeChanges(everything: boolean): Record<Attribute, boolean | null> {
    const asked = (name: Attribute) => (attrs[name] === 'keep' || (!everything && attrs[name] === startAttrs[name]) ? null : attrs[name] === 'on');
    return { readOnly: asked('readOnly'), hidden: asked('hidden'), system: asked('system'), archive: asked('archive') };
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
      ...(windows ? attributeChanges(everything) : { readOnly: null, hidden: null, system: null, archive: null }),
      modified: modifiedAt,
      accessed: accessedAt,
      recursive: everything,
    };

    const attributes = ATTRIBUTES.every((name) => asked[name] === null);
    const nothing = asked.mode === null && attributes && modifiedAt === null && accessedAt === null;
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
    <div class="special">
      {#each ATTRIBUTES as name (name)}
        <label class="check"><TriBox bind:value={attrs[name]} mixed={startAttrs[name] === 'keep'} label={LABELS[name]} />{LABELS[name]}</label>
      {/each}
    </div>
    <p class="hint">Read-only can be changed on files only. A box that shows a dash leaves that attribute as it is.</p>
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
    {#if recursive}
      <p class="hint">Everything inside gets each box as it shows now, ticked or clear; set a box to the dash to leave it alone.</p>
    {/if}
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
