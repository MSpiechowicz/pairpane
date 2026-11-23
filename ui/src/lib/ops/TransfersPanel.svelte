<script lang="ts">
  import type { JobId } from '$lib/ipc/bindings/JobId';
  import { displayName } from '$lib/panes/format';

  import { formatRate, percentDone } from './format';
  import type { JobResult, JobRow, Transfers } from './transfers.svelte';

  let { transfers, onClose }: { transfers: Transfers; onClose: () => void } = $props();

  let root = $state<HTMLElement>();
  /** The job whose errors are shown. */
  let expanded = $state<JobId | undefined>();

  const PHASES = {
    queued: 'Queued',
    preparing: 'Preparing…',
    renaming: 'Moving…',
    scanning: 'Scanning…',
    copying: 'Copying…',
    verifying: 'Verifying…',
    hashing: 'Hashing…',
    splitting: 'Splitting…',
    combining: 'Combining…',
    changing: 'Changing…',
    extracting: 'Extracting…',
    packing: 'Packing…',
    removing: 'Removing…',
    waiting: 'Waiting for your answer',
    cancelling: 'Cancelling…',
  } as const;

  const KINDS = {
    copy: 'Copy',
    move: 'Move',
    checksum: 'Checksum',
    verifyChecksum: 'Verify',
    split: 'Split',
    combine: 'Combine',
    attributes: 'Attributes',
    extract: 'Extract',
    pack: 'Pack',
    addToArchive: 'Add to archive',
    rename: 'Rename',
    sync: 'Synchronize',
    syncUndo: 'Undo synchronize',
  } as const;

  /** The jobs that put things into a folder of their own: the row says where. */
  const TARGETED: ReadonlySet<string> = new Set(['copy', 'move', 'split', 'combine', 'extract', 'pack', 'addToArchive']);

  const errorCount = (result: JobResult) => result.errors.length + result.moreErrors;

  /** What the row says about a job: how it ended, or where it is and how fast it goes. */
  function status(job: JobRow): string {
    const { result } = job;
    if (result === undefined) {
      return job.paused ? 'Paused' : formatRate(job) || PHASES[job.phase];
    }

    const parts = [{ done: 'Done', cancelled: 'Cancelled', failed: 'Failed' }[result.outcome.kind]];
    if (result.skipped > 0) {
      parts.push(`${result.skipped} skipped`);
    }

    // A check says how many files were right.
    const ok = Math.max(0, job.doneFiles - errorCount(result));
    if (job.kind === 'verifyChecksum' && ok > 0) {
      parts.push(`${ok} ok`);
    }

    if (errorCount(result) > 0) {
      parts.push(errorCount(result) === 1 ? '1 error' : `${errorCount(result)} errors`);
    }

    return parts.join(' · ');
  }

  /** Whether the row has errors to show: the job failed, or some of its items did. */
  const hasDetails = (result: JobResult) => result.outcome.kind === 'failed' || errorCount(result) > 0;

  /** Takes the focus in, gives it back when the panel goes, and closes on Escape. */
  function wire(node: HTMLElement) {
    const before = document.activeElement;
    node.focus();

    const onkeydown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onClose();
      }
    };
    node.addEventListener('keydown', onkeydown);

    return () => {
      node.removeEventListener('keydown', onkeydown);
      const lost = document.activeElement === null || document.activeElement === document.body;
      if (lost && before instanceof HTMLElement && before.isConnected) {
        before.focus();
      }
    };
  }

  // A click on whatever opened the panel is that control's own business (it toggles the panel).
  function closeOnOutside(event: MouseEvent) {
    const target = event.target;
    if (root && target instanceof Element && !root.contains(target) && !target.closest('[data-menu-opener]')) {
      onClose();
    }
  }
</script>

<svelte:window onclick={closeOnOutside} />

<section class="popup panel fade-in" data-menu aria-label="Transfers" tabindex="-1" bind:this={root} {@attach wire}>
  {#each transfers.jobs as job (job.id)}
    {@const percent = job.result?.outcome.kind === 'done' ? 100 : percentDone(job)}
    {@const result = job.result}
    <div class="job" class:leaving={job.leaving}>
      <span class="name" title={displayName(job.label)}><span class="dim">{KINDS[job.kind]}</span> {displayName(job.label)}</span>
      {#if TARGETED.has(job.kind)}
        <span class="to dim" title={displayName(job.to)}>→ {displayName(job.to)}</span>
      {:else}
        <span></span>
      {/if}
      <div
        class="h-1 overflow-hidden rounded bg-line"
        role="progressbar"
        aria-label="Progress of {displayName(job.label)}"
        aria-valuemin="0"
        aria-valuemax="100"
        aria-valuenow={percent}
      >
        <div class="h-full bg-accent" style:width={`${percent}%`}></div>
      </div>
      {#if result && hasDetails(result)}
        <button
          class="status danger"
          type="button"
          aria-expanded={expanded === job.id}
          onclick={() => (expanded = expanded === job.id ? undefined : job.id)}
        >
          {status(job)}
        </button>
      {:else}
        <span class="status">{status(job)}</span>
      {/if}
      <span class="actions">
        {#if result}
          <button class="btn" type="button" aria-label="Dismiss {displayName(job.label)}" onclick={() => transfers.dismiss(job.id)}>×</button>
        {:else}
          <button
            class="btn"
            type="button"
            aria-label="{job.paused ? 'Resume' : 'Pause'} {displayName(job.label)}"
            onclick={() => transfers.control(job.id, job.paused ? 'resume' : 'pause')}
          >
            {job.paused ? 'Resume' : 'Pause'}
          </button>
          <button
            class="btn"
            type="button"
            aria-label="Cancel {displayName(job.label)}"
            disabled={job.phase === 'cancelling'}
            onclick={() => transfers.control(job.id, 'cancel')}
          >
            Cancel
          </button>
        {/if}
      </span>
    </div>
    {#if result && expanded === job.id}
      <ul class="errors">
        {#if result.outcome.kind === 'failed'}
          <li>{displayName(result.outcome.error.message)}</li>
        {/if}
        {#each result.errors as item (item.path)}
          <li><span class="dim">{displayName(item.path)}</span> {displayName(item.error.message)}</li>
        {/each}
        {#if result.moreErrors > 0}
          <li class="dim">and {result.moreErrors} more</li>
        {/if}
      </ul>
    {/if}
  {:else}
    <p class="none">No transfers</p>
  {/each}
</section>

<style>
  .panel {
    width: min(720px, calc(100vw - 32px));
    max-height: 50vh;
    overflow-y: auto;
    pointer-events: auto;
  }

  .panel:focus {
    outline: none;
  }

  .job {
    display: grid;
    grid-template-columns: minmax(0, 1.2fr) minmax(0, 1.4fr) 80px minmax(0, 140px) auto;
    align-items: center;
    gap: 10px;
    min-height: 36px;
    padding: 0 8px;
    transition: opacity 0.3s;
  }

  .job.leaving {
    opacity: 0;
  }

  .name,
  .to,
  .status {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dim {
    color: var(--dim);
  }

  .status {
    padding: 0;
    border: 0;
    background: none;
    color: var(--dim);
    font-size: 12px;
    text-align: left;
  }

  button.status {
    cursor: pointer;
  }

  .status.danger {
    color: var(--danger);
  }

  .actions {
    display: flex;
    gap: 6px;
  }

  .errors {
    margin: 0 0 6px;
    padding: 0 8px 0 24px;
    font-size: 12px;
    color: var(--danger);
  }

  .none {
    margin: 0;
    padding: 6px 8px;
    color: var(--dim);
  }
</style>
