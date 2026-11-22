<script lang="ts">
  import type { Answer } from '$lib/ipc/bindings/Answer';
  import type { FileFacts } from '$lib/ipc/bindings/FileFacts';
  import Modal from '$lib/shell/Modal.svelte';

  import { formatFacts } from './format';
  import type { Question, Transfers } from './transfers.svelte';

  const COMPARISON = { running: 'Comparing…', identical: 'Identical', different: 'Different' } as const;

  let { transfers, question }: { transfers: Transfers; question: Question } = $props();

  const info = $derived(question.info);
  const compare = $derived(question.compare);
  let all = $state(false);

  /** Which side is the newer one: the other side has nothing to mark when the dates are equal or unknown. */
  const newer = $derived.by((): 'source' | 'target' | undefined => {
    const [source, target] = [info.source.modified, info.target.modified];
    if (source === null || target === null || source === target) {
      return undefined;
    }

    return source > target ? 'source' : 'target';
  });

  /** The Compare button shows what the comparison found, in its place. */
  const comparison = $derived(compare === undefined ? 'Compare' : COMPARISON[compare]);

  function answer(choice: Answer) {
    void transfers.answer(question, choice, choice !== 'compare' && all);
  }

  /** Escape skips this one file and nothing more: the choice that loses nothing. */
  function skipOnce() {
    void transfers.answer(question, 'skip', false);
  }
</script>

{#snippet side(label: string, facts: FileFacts, mark: boolean)}
  <div class="side">
    <span class="dim">{label}</span>
    <span>{formatFacts(facts)}</span>
    {#if mark}<span class="newer">newer</span>{/if}
  </div>
{/snippet}

<Modal title="“{info.name}” already exists" onClose={skipOnce}>
  {@render side('Source', info.source, newer === 'source')}
  {@render side('Target', info.target, newer === 'target')}

  <div class="choices">
    <button class="btn" type="button" data-autofocus onclick={() => answer('skip')}>Skip</button>
    <button class="btn" type="button" onclick={() => answer('overwrite')}>Overwrite</button>
    <button class="btn" type="button" onclick={() => answer('rename')}>Rename → {info.suggested}</button>
    <button class="btn" type="button" onclick={() => answer('overwriteOlder')}>Overwrite older</button>
    {#if info.comparable}
      <button class="btn" type="button" disabled={compare !== undefined} onclick={() => answer('compare')}>
        {comparison}
      </button>
    {/if}
  </div>

  <div class="mt-3 flex items-center justify-between gap-2">
    <label class="check"><input type="checkbox" bind:checked={all} />Apply to all</label>
    <button class="btn" type="button" onclick={() => void transfers.cancelJob(info.job)}>Cancel job</button>
  </div>
</Modal>

<style>
  .side {
    display: flex;
    align-items: baseline;
    gap: 10px;
    height: 24px;
  }

  .side > :first-child {
    width: 48px;
  }

  .dim {
    color: var(--dim);
  }

  .newer {
    color: var(--ok);
    font-size: 11px;
  }

  .choices {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 12px;
  }

  .choices > button {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
