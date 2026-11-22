<script lang="ts">
  import DialogFooter from '$lib/ops/DialogFooter.svelte';
  import { Submission } from '$lib/ops/submission.svelte';
  import { formatSize } from '$lib/panes/format';
  import Modal from '$lib/shell/Modal.svelte';

  import type { Syncer } from './syncer.svelte';

  let { syncer }: { syncer: Syncer } = $props();

  const { toRight, toLeft, overwrites } = $derived(syncer.totals);
  const submission = new Submission(() => (syncer.confirming = false));

  const count = (n: number, noun: string) => `${n.toLocaleString('en-US')} ${noun}${n === 1 ? '' : 's'}`;

  /** `3 items (1.2 KB) to the right`: one direction's part of the job. */
  const way = (total: { items: number; bytes: number }, side: string) => `${count(total.items, 'item')} (${formatSize(total.bytes)}) to the ${side}`;

  const withheld = $derived(syncer.withheld);

  const parts = $derived(
    [toRight.items > 0 ? way(toRight, 'right') : '', toLeft.items > 0 ? way(toLeft, 'left') : ''].filter((part) => part !== ''),
  );

  /** Queues the job and closes the dialog; a refusal is shown here, and the dialog stays. */
  const confirm = () =>
    submission.run(async () => {
      await syncer.apply();
      syncer.close();
    });
</script>

<Modal title="Synchronize {count(toRight.items + toLeft.items, 'item')}?" onClose={submission.close}>
  <p class="note">Copy {parts.join(' and ')}.</p>
  <p class="note">
    {overwrites > 0 ? `${count(overwrites, 'file')} will be overwritten. This cannot be undone.` : 'Nothing will be overwritten.'}
    Nothing is deleted.
  </p>
  {#if withheld > 0}
    <p class="note">
      {count(withheld, 'row')} with an action {withheld === 1 ? 'is' : 'are'} hidden by the filters and will not be synchronized.
    </p>
  {/if}
  <DialogFooter
    failure={submission.failure}
    busy={submission.busy}
    confirm="Synchronize"
    danger={overwrites > 0}
    focus={overwrites > 0 ? 'cancel' : 'confirm'}
    onConfirm={confirm}
    onCancel={submission.close}
  />
</Modal>

<style>
  .note {
    margin: 0 0 6px;
  }
</style>
