<script lang="ts">
  import DialogFooter from '$lib/ops/DialogFooter.svelte';
  import { Submission } from '$lib/ops/submission.svelte';
  import { displayName, formatSize } from '$lib/panes/format';
  import VirtualRows from '$lib/panes/VirtualRows.svelte';
  import Modal from '$lib/shell/Modal.svelte';

  import { itemCount } from './plan';
  import type { Syncer } from './syncer.svelte';

  let { syncer }: { syncer: Syncer } = $props();

  const totals = $derived(syncer.totals);
  const { toRight, toLeft, overwrites, removeLeft, removeRight } = $derived(totals);
  const removing = $derived(removeLeft.items + removeRight.items);
  const removed = $derived(syncer.removed);
  const submission = new Submission(() => (syncer.confirming = false));
  /** Deleting for good is asked about a second time, on its own. */
  let second = $state(false);

  const count = (n: number, noun: string) => `${n.toLocaleString('en-US')} ${noun}${n === 1 ? '' : 's'}`;

  /** `3 items (1.2 KB) to the right`: one side's part of the job. */
  const way = (total: { items: number; bytes: number }, side: string) => `${count(total.items, 'item')} (${formatSize(total.bytes)}) ${side}`;

  const withheld = $derived(syncer.withheld);

  const copies = $derived(
    [toRight.items > 0 ? way(toRight, 'to the right') : '', toLeft.items > 0 ? way(toLeft, 'to the left') : ''].filter((part) => part !== ''),
  );

  const removals = $derived(
    [removeLeft.items > 0 ? way(removeLeft, 'from the left') : '', removeRight.items > 0 ? way(removeRight, 'from the right') : ''].filter(
      (part) => part !== '',
    ),
  );

  /** Queues the job and closes the dialog; a refusal is shown here, and the dialog stays. */
  const run = () =>
    submission.run(async () => {
      await syncer.apply();
      syncer.close();
    });

  /** The first yes: deleting for good asks once more before anything is queued. */
  function confirm() {
    if (syncer.permanent && removing > 0 && !second) {
      second = true;
    } else {
      void run();
    }
  }

  function cancel() {
    if (second && !submission.busy) {
      second = false;
    } else {
      submission.close();
    }
  }
</script>

{#if second}
  <Modal title="Delete {count(removing, 'item')} permanently?" onClose={cancel}>
    <p class="note">
      {count(removing, 'item')} will be deleted for good: {removals.join(' and ')}. They do not go to the Trash and cannot be restored.
    </p>
    <DialogFooter
      failure={submission.failure}
      busy={submission.busy}
      confirm="Delete permanently"
      cancel="Back"
      danger
      focus="cancel"
      onConfirm={confirm}
      onCancel={cancel}
    />
  </Modal>
{:else}
  <Modal title="Synchronize {count(itemCount(totals), 'item')}?" onClose={cancel}>
    {#if copies.length > 0}
      <p class="note">Copy {copies.join(' and ')}.</p>
      <p class="note">
        {overwrites > 0 ? `${count(overwrites, 'file')} will be overwritten. This cannot be undone.` : 'Nothing will be overwritten.'}
      </p>
    {/if}
    {#if removing > 0}
      <p class="note">
        Remove {removals.join(' and ')}, {syncer.permanent ? 'permanently' : 'to the Trash'}. Items that changed since the comparison
        are kept.
      </p>
      <div class="removed" role="list" aria-label="Items to remove" style:--rows={removed.length}>
        <VirtualRows count={removed.length} rowHeight={20}>
          {#snippet row(index)}
            {@const item = removed[index]}
            <div class="item" role="listitem" title={displayName(item.path)}>
              <span class="side">{item.side === 'left' ? 'Left' : 'Right'}</span>
              {displayName(item.path)}
            </div>
          {/snippet}
        </VirtualRows>
      </div>
      <label class="check">
        <input type="checkbox" disabled={submission.busy} bind:checked={syncer.permanent} />Delete permanently instead of moving to the Trash
      </label>
    {:else}
      <p class="note">Nothing is deleted.</p>
    {/if}
    {#if withheld > 0}
      <p class="note">
        {count(withheld, 'row')} with an action {withheld === 1 ? 'is' : 'are'} hidden by the filters and will not be synchronized.
      </p>
    {/if}
    <DialogFooter
      failure={submission.failure}
      busy={submission.busy}
      confirm={syncer.permanent && removing > 0 ? 'Continue…' : 'Synchronize'}
      danger={overwrites > 0 || removing > 0}
      focus={overwrites > 0 || removing > 0 ? 'cancel' : 'confirm'}
      onConfirm={confirm}
      onCancel={cancel}
    />
  </Modal>
{/if}

<style>
  .note {
    margin: 0 0 6px;
  }

  .removed {
    height: min(160px, calc(var(--rows) * 20px + 2px));
    margin: 0 0 8px;
    border: 1px solid var(--line);
    border-radius: 6px;
    font-size: 12px;
  }

  .item {
    height: 20px;
    padding: 0 8px;
    overflow: hidden;
    line-height: 20px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .side {
    color: var(--dim);
  }

  .check {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 6px;
  }
</style>
