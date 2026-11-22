<script lang="ts">
  import { untrack } from 'svelte';

  import type { RenameRule } from '$lib/ipc/bindings/RenameRule';
  import type { Transport } from '$lib/ipc/transport';
  import DialogFooter from '$lib/ops/DialogFooter.svelte';
  import { what } from '$lib/ops/dialogs';
  import { enterSubmits, Submission } from '$lib/ops/submission.svelte';
  import { displayName } from '$lib/panes/format';
  import SavedBar from '$lib/saved/SavedBar.svelte';
  import Modal from '$lib/shell/Modal.svelte';

  import PreviewTable from './PreviewTable.svelte';
  import { Renamer } from './renamer.svelte';
  import RuleFields from './RuleFields.svelte';
  import type { SavedRenames } from './saved.svelte';
  import '../ops/fields.css';

  let {
    renames,
    ipc,
    path,
    names,
    onClose,
    onDone,
  }: {
    /** The rules kept by name. */
    renames: SavedRenames;
    ipc: Transport;
    /** The folder the items are in. */
    path: string;
    names: string[];
    onClose: () => void;
    /** The job started. */
    onDone: () => void;
  } = $props();

  // The items and the folder are those the dialog was opened for.
  const renamer = untrack(() => new Renamer(ipc, path, names));
  renamer.watch();
  const submission = new Submission(() => onClose());

  const count = (n: number, noun: string) => `${n.toLocaleString('en-US')} ${noun}${n === 1 ? '' : 's'}`;

  /** The line under the table: what the preview says, or why there is none. */
  const status = $derived.by(() => {
    if (renamer.failure !== undefined) {
      return { text: renamer.failure, alert: true };
    }

    if (renamer.pending) {
      return { text: 'Checking the names…', alert: false };
    }

    if (renamer.error !== undefined) {
      return { text: 'The rule has a mistake: the names are not shown.', alert: false };
    }

    const parts = [count(renamer.rows.length, 'item'), `${renamer.changed.length.toLocaleString('en-US')} to rename`];
    if (renamer.problems > 0) {
      parts.push(count(renamer.problems, 'problem'));
    }

    return { text: parts.join(' · '), alert: false };
  });

  /** What Save as keeps: the rule, unless it is faulty (the backend drops such a rule when it is loaded). */
  function savable(): RenameRule | string {
    return renamer.error === undefined ? ($state.snapshot(renamer.rule) as RenameRule) : 'Fix the rule before saving it.';
  }

  async function submit() {
    if (submission.busy || !renamer.canApply) {
      return;
    }

    await submission.run(async () => {
      await renamer.apply();
      onDone();
    });
  }
</script>

<Modal title="Rename {what(names)}" variant="wide" onClose={submission.close}>
  <div class="body" {@attach enterSubmits(submit)}>
    <SavedBar saved={renames} current={savable} onLoad={(rule) => renamer.load(rule)} />
    <RuleFields {renamer} />
    <PreviewTable rows={renamer.rows} stale={renamer.pending} />
    {#if status.alert}
      <p class="status failure" role="alert">{displayName(status.text)}</p>
    {:else}
      <p class="status" role="status">{status.text}</p>
    {/if}
  </div>
  <DialogFooter failure={submission.failure} busy={submission.busy} disabled={!renamer.canApply} confirm="Rename" onConfirm={submit} onCancel={submission.close} />
</Modal>

<style>
  .body {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .status {
    margin: 0;
    color: var(--dim);
  }

  .failure {
    color: var(--danger);
  }
</style>
