<script lang="ts">
  import type { PairingShown } from '$lib/ipc/bindings/PairingShown';
  import '$lib/ops/fields.css';

  import { countdown } from './format';
  import QrCode from './QrCode.svelte';

  let {
    offer,
    remaining,
    onCancel,
  }: {
    /** The code this device offers. */
    offer: PairingShown;
    /** The milliseconds the code has left. */
    remaining: number;
    /** Withdraws the code. */
    onCancel: () => void;
  } = $props();

  /** What the Copy button last did: told under it, as the button itself does not change. */
  let copy = $state<string>();

  async function copyTarget() {
    try {
      await navigator.clipboard.writeText(offer.target);
      copy = 'Copied.';
    } catch {
      copy = 'This system does not allow copying here; select the address and copy it by hand.';
    }
  }
</script>

<section class="group panel" aria-label="Pair a device">
  <p class="steps">On the other device choose “Join with a code”, then type this code and give it the address below.</p>
  <div class="body">
    <div class="shown">
      <p class="code" aria-label="Pairing code">{offer.code}</p>
      <p class="left" role="timer" aria-label="Time left for the code">Expires in {countdown(remaining)}</p>
      <label class="labelled">
        Address
        <input class="field" type="text" readonly value={offer.target} onfocus={(event) => event.currentTarget.select()} />
      </label>
      <div class="buttons">
        <button class="btn" type="button" onclick={copyTarget}>Copy address</button>
        <button class="btn" type="button" onclick={onCancel}>Cancel</button>
      </div>
      {#if copy}
        <p class="hint" role="status">{copy}</p>
      {/if}
    </div>
    {#if offer.qr}
      <QrCode qr={offer.qr} />
    {/if}
  </div>
</section>

<style>
  .panel {
    flex-direction: column;
    align-items: stretch;
  }

  .steps,
  .code,
  .left {
    margin: 0;
  }

  .steps {
    color: var(--dim);
  }

  .body {
    display: flex;
    flex-wrap: wrap;
    gap: 14px;
  }

  .shown {
    display: flex;
    flex: 1 1 280px;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }

  .code {
    font: 600 26px var(--font-mono);
    letter-spacing: 0.08em;
    user-select: all;
  }

  .left {
    color: var(--dim);
  }

  .buttons {
    display: flex;
    gap: 8px;
  }
</style>
