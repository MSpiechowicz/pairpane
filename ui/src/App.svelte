<script lang="ts">
  import { onMount } from 'svelte';

  import { ipc } from '$lib/ipc/client';
  import type { AppInfo } from '$lib/ipc/bindings/AppInfo';

  let info = $state<AppInfo | undefined>();
  let failure = $state<string | undefined>();

  onMount(async () => {
    try {
      info = await ipc.invoke('app_info', undefined);
    } catch (error) {
      failure = error instanceof Error ? error.message : String(error);
    }
  });
</script>

<main class="flex h-full flex-col items-center justify-center gap-3 bg-bg text-fg">
  <img class="fade-in" src="/favicon.svg" alt="" width="64" height="64" />
  <h1 class="fade-in m-0 text-2xl font-semibold">Pairpane</h1>

  {#if info}
    <p class="fade-in m-0 font-mono text-xs text-dim">v{info.version}</p>
  {:else if failure}
    <p class="m-0 text-sm text-danger" role="alert">Could not load app info: {failure}</p>
  {/if}
</main>

<style>
  .fade-in {
    animation: fade-in 300ms ease-out;
  }

  @keyframes fade-in {
    from {
      opacity: 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .fade-in {
      animation: none;
    }
  }
</style>
