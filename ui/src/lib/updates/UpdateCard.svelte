<script lang="ts">
  import type { Updates } from './updates.svelte';

  let { updates }: { updates: Updates } = $props();

  const state = $derived(updates.state);

  /** Download progress in percent, when the size is known. */
  const percent = $derived.by(() => {
    if (state.kind !== 'installing' || !state.progress?.total) {
      return undefined;
    }

    return Math.min(100, Math.round((state.progress.downloaded / state.progress.total) * 100));
  });

  // "Up to date" is only a confirmation of a manual check: it goes away by itself.
  $effect(() => {
    if (state.kind !== 'upToDate') {
      return;
    }

    const timer = setTimeout(() => updates.dismiss(), 4000);

    return () => clearTimeout(timer);
  });
</script>

{#if state.kind !== 'idle'}
  <section
    class="update-card fixed right-4 bottom-4 w-80 rounded-lg border border-line bg-panel p-4 text-sm text-fg shadow-lg"
    aria-live="polite"
    aria-label="Updates"
  >
    {#if state.kind === 'checking'}
      <p class="m-0 text-dim">Checking for updates…</p>
    {:else if state.kind === 'upToDate'}
      <p class="m-0">
        Pairpane is up to date{#if state.version}&nbsp;<span class="font-mono text-dim">(v{state.version})</span>{/if}.
      </p>
    {:else if state.kind === 'available'}
      <p class="m-0 font-semibold">Pairpane {state.info.version} is available</p>
      <p class="mt-1 mb-0 text-dim">You have {state.info.currentVersion}.</p>
      {#if state.info.notes}
        <p class="notes mt-2 mb-0 max-h-32 overflow-auto text-xs whitespace-pre-line text-dim">{state.info.notes}</p>
      {/if}
      <div class="mt-3 flex justify-end gap-2">
        <button class="btn" type="button" onclick={() => updates.dismiss()}>Later</button>
        <button class="btn btn-primary" type="button" onclick={() => updates.install()}>Install</button>
      </div>
    {:else if state.kind === 'installing'}
      <p class="m-0">
        {state.progress?.phase === 'installing' ? 'Installing' : 'Downloading'} Pairpane {state.info.version}…
      </p>
      <div
        class="mt-2 h-1 overflow-hidden rounded bg-line"
        role="progressbar"
        aria-valuemin="0"
        aria-valuemax="100"
        aria-valuenow={percent}
      >
        <div class="h-full bg-accent transition-[width]" style:width={`${percent ?? 0}%`}></div>
      </div>
    {:else if state.kind === 'ready'}
      <p class="m-0 font-semibold">Pairpane {state.info.version} is installed</p>
      <p class="mt-1 mb-0 text-dim">Restart to start using it.</p>
      <div class="mt-3 flex justify-end gap-2">
        <button class="btn" type="button" onclick={() => updates.dismiss()}>Later</button>
        <button class="btn btn-primary" type="button" onclick={() => updates.relaunch()}>Restart</button>
      </div>
    {:else if state.kind === 'failed'}
      <p class="m-0 text-danger" role="alert">{state.message}</p>
      <div class="mt-3 flex justify-end gap-2">
        <button class="btn" type="button" onclick={() => updates.dismiss()}>Close</button>
        {#if state.info}
          <button class="btn btn-primary" type="button" onclick={() => updates.install()}>Try again</button>
        {/if}
      </div>
    {/if}
  </section>
{/if}

<style>
  .btn {
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--panel-2);
    color: var(--text);
    padding: 4px 12px;
    cursor: pointer;
  }

  .btn:hover {
    border-color: var(--accent);
  }

  .btn-primary {
    border-color: var(--accent);
    background: var(--accent);
    color: #fff;
  }

  .update-card {
    animation: fade-in 200ms ease-out;
  }

  @keyframes fade-in {
    from {
      opacity: 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .update-card {
      animation: none;
    }
  }
</style>
