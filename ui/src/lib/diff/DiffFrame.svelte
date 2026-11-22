<script lang="ts">
  import type { Snippet } from 'svelte';

  let { children }: { children: Snippet } = $props();
</script>

<div class="diff">
  {@render children()}
</div>

<style>
  .diff {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
  }
</style>
