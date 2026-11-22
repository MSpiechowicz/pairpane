<script lang="ts">
  import type { CompareRoot } from '$lib/ipc/bindings/CompareRoot';
  import DialogFooter from '$lib/ops/DialogFooter.svelte';
  import { archivePlace } from '$lib/ops/dialogs';
  import { enterSubmits } from '$lib/ops/submission.svelte';
  import { displayName, formatSize } from '$lib/panes/format';
  import Modal from '$lib/shell/Modal.svelte';

  import { STATUS_TEXT } from './plan';
  import SyncConfirm from './SyncConfirm.svelte';
  import SyncTable from './SyncTable.svelte';
  import type { Roots, Syncer, SyncSummary } from './syncer.svelte';
  import '../ops/fields.css';

  let { syncer, roots }: { syncer: Syncer; roots: Roots } = $props();

  const STATUSES = ['leftOnly', 'rightOnly', 'leftNewer', 'rightNewer', 'differs', 'equal'] as const;

  const totals = $derived(syncer.totals);
  const planned = $derived(totals.toRight.items + totals.toLeft.items);

  const count = (n: number, noun: string) => `${n.toLocaleString('en-US')} ${noun}${n === 1 ? '' : 's'}`;

  /** A side as the dialog names it: a folder, or a folder inside an archive. */
  function place(root: CompareRoot): string {
    return root.kind === 'folder' ? root.path : archivePlace(root.archive, root.dir);
  }

  function startOrStop() {
    if (syncer.running) {
      syncer.stop();
    } else {
      void syncer.start();
    }
  }

  /** The line under the Start button once the scan is over. */
  function summary({ rows, skipped, truncated, cancelled }: SyncSummary): string {
    const parts = [`${cancelled ? 'Stopped: ' : ''}${count(rows, 'row')}`];
    if (skipped > 0) {
      parts.push(`${skipped.toLocaleString('en-US')} skipped (unreadable, unusable or alike names)`);
    }

    if (truncated) {
      parts.push('the scan stopped at its limit: narrow it to see more');
    }

    return parts.join(' · ');
  }

  /** The line under the Start button while the scan runs. */
  function progress(): string {
    const seen = syncer.progress;
    if (seen === undefined) {
      return 'Comparing…';
    }

    return `Comparing… ${count(seen.folders, 'folder')} · ${count(seen.files, 'file')} · ${formatSize(seen.bytes)}`;
  }
</script>

<Modal title="Synchronize folders" variant="wide" onClose={() => syncer.close()}>
  <div class="sync" {@attach enterSubmits(startOrStop)}>
    <p class="place">Left <span class="dialog-path">{displayName(place(roots.left))}</span></p>
    <p class="place">Right <span class="dialog-path">{displayName(place(roots.right))}</span></p>

    <fieldset class="group">
      <legend>Compare</legend>
      <label class="check"><input type="checkbox" disabled={syncer.running} bind:checked={syncer.subfolders} />Subfolders</label>
      <label class="check">
        <input type="checkbox" disabled={syncer.running || !syncer.contentAllowed} bind:checked={syncer.content} />By content
      </label>
      <label class="check"><input type="checkbox" disabled={syncer.running} bind:checked={syncer.hidden} />Hidden files</label>
      {#if !syncer.contentAllowed}
        <span class="note">Files in an archive are not compared by content.</span>
      {/if}
    </fieldset>

    <div class="bar">
      <button class="btn btn-primary" type="button" disabled={syncer.stopping} data-autofocus onclick={startOrStop}>
        {syncer.running ? 'Stop' : 'Start'}
      </button>
      {#if syncer.failure}
        <p class="failure" role="alert">{displayName(syncer.failure)}</p>
      {:else if syncer.running}
        <p class="status">{progress()}</p>
      {:else if syncer.summary}
        <p class="status" role="status">{summary(syncer.summary)}</p>
      {/if}
    </div>

    <fieldset class="group">
      <legend>Show</legend>
      {#each STATUSES as status (status)}
        <label class="check">
          <input type="checkbox" bind:checked={syncer.shown[status]} />{STATUS_TEXT[status]} ({syncer.counts[status].toLocaleString('en-US')})
        </label>
      {/each}
      <label class="labelled">
        <span>Names</span>
        <input
          class="field"
          type="text"
          spellcheck="false"
          autocomplete="off"
          placeholder="*.txt; *.md (blank: every name)"
          bind:value={syncer.nameFilter}
        />
      </label>
    </fieldset>

    <SyncTable {syncer} />

    <div class="bulk">
      <span>Picked: {syncer.picked.toLocaleString('en-US')}</span>
      <button class="btn" type="button" disabled={syncer.picked === 0} aria-label="Copy picked rows to the right" onclick={() => syncer.chooseAll('toRight')}>→</button>
      <button class="btn" type="button" disabled={syncer.picked === 0} aria-label="Copy picked rows to the left" onclick={() => syncer.chooseAll('toLeft')}>←</button>
      <button class="btn" type="button" disabled={syncer.picked === 0} aria-label="Skip picked rows" onclick={() => syncer.chooseAll('skip')}>Skip</button>
      {#if syncer.notice}
        <span class="note" role="status">{displayName(syncer.notice)}</span>
      {/if}
    </div>

    <p class="totals" role="status">
      {#if planned === 0}
        Nothing is to be copied.
      {:else}
        → {count(totals.toRight.items, 'item')} · {formatSize(totals.toRight.bytes)} &nbsp; ← {count(totals.toLeft.items, 'item')} · {formatSize(totals.toLeft.bytes)}
      {/if}
    </p>
  </div>

  <DialogFooter busy={false} disabled={syncer.running || planned === 0} confirm="Synchronize" cancel="Close" onConfirm={() => syncer.ask()} onCancel={() => syncer.close()} />
</Modal>

{#if syncer.confirming}
  <SyncConfirm {syncer} />
{/if}

<style>
  .sync {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .place {
    margin: 0;
    color: var(--dim);
  }

  .bar,
  .bulk {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .bar p {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .bar {
    gap: 12px;
  }

  .status,
  .note,
  .totals {
    color: var(--dim);
  }

  .totals {
    margin: 0;
  }

  .failure {
    color: var(--danger);
  }
</style>
