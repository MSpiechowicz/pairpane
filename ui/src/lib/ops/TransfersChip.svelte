<script lang="ts">
  import type { Transfers } from './transfers.svelte';

  let { transfers, open, onToggle }: { transfers: Transfers; open: boolean; onToggle: () => void } = $props();

  const overall = $derived(transfers.overall);
  const label = $derived(`⇅ ${overall.count}${overall.percent === undefined ? '' : ` · ${overall.percent} %`}`);
</script>

<!-- In the dock, only while there are jobs to show; the panel it opens is the place for everything else. -->
{#if overall.count > 0}
  <button
    class="dock-key fade-in"
    type="button"
    data-menu-opener
    aria-label="Transfers (Ctrl+J)"
    aria-expanded={open}
    onclick={onToggle}
  >
    {label}
  </button>
{/if}

<style>
  button {
    padding: 0 10px;
    color: var(--accent-2);
    font-variant-numeric: tabular-nums;
  }
</style>
