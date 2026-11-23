<script lang="ts">
  // Test helper: what starts an effect (`watch()`, `followPreferences`) has to run while a component is being set up.
  import { untrack } from 'svelte';

  let { start }: { start: () => void } = $props();
  untrack(() => start)();
</script>
