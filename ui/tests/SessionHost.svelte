<script lang="ts">
  // Test helper: `watch()` starts effects, so it has to run while a component is being set up.
  import { untrack } from 'svelte';

  import type { Session } from '$lib/shell/session.svelte';

  let { session }: { session: Session } = $props();
  untrack(() => session).watch();
</script>
