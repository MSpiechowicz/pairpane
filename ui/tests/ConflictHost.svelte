<script lang="ts">
  // Test helper: shows the conflict dialog for the store's first question, as the app does.
  import ConflictDialog from '$lib/ops/ConflictDialog.svelte';
  import type { Transfers } from '$lib/ops/transfers.svelte';

  let { transfers }: { transfers: Transfers } = $props();
  const question = $derived(transfers.question);
</script>

{#if question}
  <ConflictDialog {transfers} {question} />
{/if}
