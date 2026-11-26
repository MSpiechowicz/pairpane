<script lang="ts">
  import type { PairedDevice } from '$lib/ipc/bindings/PairedDevice';
  import ChoiceDialog from '$lib/shell/ChoiceDialog.svelte';
  import { displayName } from '$lib/panes/format';
  import '$lib/ops/fields.css';

  import { shortId, statusText, type Status } from './format';

  let {
    device,
    status,
    onRename,
    onUnpair,
  }: {
    device: PairedDevice;
    status: Status;
    /** Gives the device a name; whether it was done. */
    onRename: (name: string) => Promise<boolean>;
    onUnpair: () => void;
  } = $props();

  /** The name being typed; none while the row is not being renamed. */
  let draft = $state<string>();
  let saving = $state(false);
  let asking = $state(false);

  async function save() {
    const name = draft?.trim();
    if (name === undefined || saving) {
      return;
    }

    if (name === '' || name === device.name) {
      draft = undefined;
      return;
    }

    saving = true;
    const done = await onRename(name);
    saving = false;
    if (done) {
      draft = undefined;
    }
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'Enter') {
      event.preventDefault();
      void save();
    } else if (event.key === 'Escape') {
      // Used up here: it ends the renaming, and the dialog stays.
      event.preventDefault();
      draft = undefined;
    }
  }
</script>

<li class="row">
  <div class="who">
    {#if draft === undefined}
      <span class="name">{displayName(device.name)}</span>
    {:else}
      <input class="field" type="text" aria-label="New name of {displayName(device.name)}" bind:value={draft} disabled={saving} {onkeydown} {@attach (node) => node.focus()} />
    {/if}
    <span class="id" title={device.nodeId}>{shortId(device.nodeId)}</span>
  </div>
  <span class="status" class:online={status !== 'checking' && status.kind === 'online'}>{displayName(statusText(status, device.name))}</span>
  <div class="buttons">
    {#if draft === undefined}
      <button class="btn" type="button" aria-label="Rename {displayName(device.name)}" onclick={() => (draft = device.name)}>Rename</button>
      <button class="btn" type="button" aria-label="Unpair {displayName(device.name)}" onclick={() => (asking = true)}>Unpair</button>
    {:else}
      <button class="btn btn-primary" type="button" disabled={saving} onclick={save}>Save</button>
      <button class="btn" type="button" disabled={saving} onclick={() => (draft = undefined)}>Cancel</button>
    {/if}
  </div>
</li>

{#if asking}
  <ChoiceDialog
    title="Unpair {device.name}?"
    message="This device forgets it. To use it again, pair the two devices again."
    confirm="Unpair"
    danger
    onConfirm={() => ((asking = false), onUnpair())}
    onCancel={() => (asking = false)}
  />
{/if}

<style>
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 14px;
    padding: 6px 0;
    border-top: 1px solid var(--line);
  }

  .row:first-child {
    border-top: 0;
  }

  .who {
    display: flex;
    flex: 1 1 220px;
    align-items: baseline;
    gap: 10px;
    min-width: 0;
  }

  .who .field {
    flex: 1;
    min-width: 0;
  }

  .name {
    overflow-wrap: anywhere;
  }

  .id,
  .status {
    color: var(--dim);
  }

  .id {
    font: 12px var(--font-mono);
  }

  .status.online {
    color: var(--ok);
  }

  .buttons {
    display: flex;
    gap: 8px;
  }
</style>
