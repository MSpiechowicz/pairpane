<script lang="ts">
  import type { Comparison } from './controller.svelte';
  import SyncDialog from './SyncDialog.svelte';

  /** What the comparison of the panes shows: the Synchronize dialog, while it is open. */
  let { comparison }: { comparison: Comparison } = $props();

  const { syncer } = $derived(comparison);
</script>

{#if syncer.roots !== undefined}
  <SyncDialog {syncer} roots={syncer.roots} />
{/if}
