<script lang="ts">
  import { displayName } from '$lib/panes/format';

  import { ENDING_LABELS, type EditSession } from './session.svelte';

  let { session }: { session: EditSession } = $props();
</script>

{#if session.readOnly}
  <p class="banner" role="status">This file is read-only: it cannot be changed or saved here.</p>
{/if}
{#if session.mixedEndings}
  <p class="banner" role="status">
    This file mixes line endings; saving writes all of them as {ENDING_LABELS[session.lineEnding]}.
  </p>
{/if}
{#if session.note}
  <p class="banner" role="status">{displayName(session.note)}</p>
{/if}
{#if session.failure}
  <p class="banner error" role="alert">{displayName(session.failure)}</p>
{/if}

<style>
  .banner {
    margin: 0;
    padding: 6px 12px;
    border-bottom: 1px solid var(--line);
    background: var(--panel-2);
    color: var(--dim);
    font-size: 12px;
  }

  .banner.error {
    color: var(--danger);
  }
</style>
