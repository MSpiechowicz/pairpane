<script lang="ts">
  import { onMount } from 'svelte';

  import type { Transport } from '$lib/ipc/transport';
  import DialogFooter from '$lib/ops/DialogFooter.svelte';
  import { displayName } from '$lib/panes/format';
  import Modal from '$lib/shell/Modal.svelte';
  import '$lib/ops/fields.css';

  import { Devices } from './devices.svelte';
  import DeviceRow from './DeviceRow.svelte';
  import { keyStorageNote, shortId, TRAY_NOTE } from './format';
  import JoinPanel from './JoinPanel.svelte';
  import PairPanel from './PairPanel.svelte';

  let { ipc, onClose }: { ipc: Transport; onClose: () => void } = $props();

  // The transport does not change while the dialog is open.
  // svelte-ignore state_referenced_locally
  const devices = new Devices(ipc);

  // The screen is open while the dialog is: the code it offers is withdrawn and the timers stop when it goes.
  onMount(() => {
    void devices.open();

    return () => devices.close();
  });
</script>

<Modal title="Devices" variant="wide" {onClose}>
  <div class="sections">
    {#if devices.error !== undefined}
      <p class="failure" role="alert">{displayName(devices.error)}</p>
      <div><button class="btn" type="button" onclick={() => devices.open()}>Try again</button></div>
    {:else if devices.overview === undefined}
      <p class="note">Opening…</p>
    {:else}
      {@const { own } = devices.overview}
      <fieldset class="group own">
        <legend>This device</legend>
        <p class="line"><span class="name">{displayName(own.name)}</span> <span class="id" title={own.nodeId}>{shortId(own.nodeId)}</span></p>
        <p class="note">{keyStorageNote(own.keyStorage)}</p>
        <p class="note">{TRAY_NOTE}</p>
      </fieldset>

      <div class="modes">
        <button class="btn" type="button" aria-pressed={devices.mode === 'pair'} onclick={() => devices.choose('pair')}>Pair a device</button>
        <button class="btn" type="button" aria-pressed={devices.mode === 'join'} onclick={() => devices.choose('join')}>Join with a code</button>
      </div>

      {#if devices.mode === 'pair'}
        {#if devices.offer}
          <PairPanel offer={devices.offer} remaining={devices.remaining} onCancel={() => devices.closePanel()} />
        {:else}
          <p class="note">Getting a code…</p>
        {/if}
      {:else if devices.mode === 'join'}
        <JoinPanel found={devices.found} busy={devices.busy} onJoin={(code, target) => devices.join(code, target)} onCancel={() => devices.closePanel()} />
      {/if}

      {#if devices.notice && !devices.notice.bad}
        <p class="note" role="status">{displayName(devices.notice.text)}</p>
      {/if}

      <fieldset class="group paired">
        <legend>Paired devices</legend>
        {#if devices.overview.devices.length === 0}
          <p class="note">No device is paired yet.</p>
        {:else}
          <ul class="list" aria-label="Paired devices">
            {#each devices.overview.devices as device (device.nodeId)}
              <DeviceRow
                {device}
                status={devices.statuses[device.nodeId] ?? 'checking'}
                onRename={(name) => devices.rename(device.nodeId, name)}
                onUnpair={() => devices.unpair(device.nodeId)}
              />
            {/each}
          </ul>
        {/if}
      </fieldset>
    {/if}
  </div>

  <DialogFooter busy={false} cancel="Close" focus="cancel" failure={devices.notice?.bad ? devices.notice.text : undefined} onCancel={onClose} />
</Modal>

<style>
  .sections {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-height: min(560px, 68vh);
    overflow: auto;
  }

  .note,
  .line,
  .failure {
    flex-basis: 100%;
    margin: 0;
  }

  .note {
    color: var(--dim);
  }

  .failure {
    color: var(--danger);
  }

  .own {
    flex-direction: column;
    align-items: stretch;
    gap: 4px;
  }

  .id {
    color: var(--dim);
    font: 12px var(--font-mono);
  }

  .modes {
    display: flex;
    gap: 8px;
  }

  .modes .btn[aria-pressed='true'] {
    border-color: var(--accent);
  }

  .paired {
    display: block;
  }

  .list {
    margin: 0;
    padding: 0;
    list-style: none;
  }
</style>
