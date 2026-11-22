<script lang="ts">
  import QuestionDialog from '$lib/editor/QuestionDialog.svelte';
  import Modal from '$lib/shell/Modal.svelte';

  import type { FileDiff } from './fileDiff.svelte';
  import HexDiffView from './HexDiffView.svelte';
  import TextDiffView from './TextDiffView.svelte';

  let { diff }: { diff: FileDiff } = $props();
</script>

{#if diff.pair}
  {@const pair = diff.pair}
  <Modal title="Compare {pair.left.name} and {pair.right.name}" variant="full" onClose={() => void diff.requestClose()}>
    <TextDiffView {diff} {pair} />
  </Modal>
{:else if diff.hex}
  {@const hex = diff.hex}
  <Modal title="Compare {hex.left.name} and {hex.right.name}" variant="full" onClose={() => void diff.requestClose()}>
    <HexDiffView {diff} {hex} />
  </Modal>
{/if}

{#if diff.question}
  <QuestionDialog question={diff.question} />
{/if}
