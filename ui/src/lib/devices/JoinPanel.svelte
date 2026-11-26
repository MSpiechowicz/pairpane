<script lang="ts">
  import type { FoundDevice } from '$lib/ipc/bindings/FoundDevice';
  import { displayName } from '$lib/panes/format';
  import '$lib/ops/fields.css';

  import { shortId } from './format';

  let {
    found,
    busy,
    onJoin,
    onCancel,
  }: {
    /** The devices on the network that offer to pair. */
    found: readonly FoundDevice[];
    /** A join is on its way. */
    busy: boolean;
    onJoin: (code: string, target: string) => void;
    onCancel: () => void;
  } = $props();

  let target = $state('');
  let code = $state('');
</script>

<form
  class="group panel"
  aria-label="Join with a code"
  onsubmit={(event) => {
    event.preventDefault();
    onJoin(code, target);
  }}
>
  <p class="steps">On the other device choose “Pair a device”. Pick it below or paste its address, and type the code it shows.</p>
  {#if found.length > 0}
    <ul class="found" aria-label="Devices found on the network">
      {#each found as device (device.nodeId)}
        <li>
          <span class="name">{device.name === null ? 'Unnamed' : displayName(device.name)}</span>
          <span class="id" title={device.nodeId}>{shortId(device.nodeId)}</span>
          {#if device.paired}
            <span class="badge">paired</span>
          {:else}
            <button class="btn" type="button" disabled={busy} aria-label="Use {device.name === null ? shortId(device.nodeId) : displayName(device.name)}" onclick={() => (target = device.nodeId)}>Use</button>
          {/if}
        </li>
      {/each}
    </ul>
    <p class="hint">Anyone on the network can announce any name; check the id.</p>
  {:else}
    <p class="hint">Looking for devices on the network…</p>
  {/if}
  <label class="labelled">
    Address or id
    <input class="field" type="text" bind:value={target} disabled={busy} autocomplete="off" spellcheck="false" />
  </label>
  <label class="labelled">
    Code
    <input class="field" type="text" bind:value={code} disabled={busy} autocomplete="off" spellcheck="false" />
  </label>
  <div class="buttons">
    <button class="btn btn-primary" type="submit" disabled={busy || target.trim() === '' || code.trim() === ''}>{busy ? 'Connecting…' : 'Join'}</button>
    <button class="btn" type="button" disabled={busy} onclick={onCancel}>Cancel</button>
  </div>
</form>

<style>
  .panel {
    flex-direction: column;
    align-items: stretch;
  }

  .steps,
  .hint {
    margin: 0;
    color: var(--dim);
  }

  .found {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .found li {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .name {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .id {
    color: var(--dim);
    font: 12px var(--font-mono);
  }

  .badge {
    color: var(--dim);
    font-size: 11.5px;
  }

  .buttons {
    display: flex;
    gap: 8px;
  }
</style>
