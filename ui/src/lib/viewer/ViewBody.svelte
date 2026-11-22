<script lang="ts">
  import { displayName } from '$lib/panes/format';

  import HexView from './HexView.svelte';
  import { PREVIEWS } from './previews';
  import type { ViewSearch } from './search.svelte';
  import type { ViewSession } from './session.svelte';
  import TextView from './TextView.svelte';

  let {
    session,
    search,
    focus = false,
  }: {
    session: ViewSession;
    /** The search whose match the text and hex views show (the quick view has none). */
    search?: ViewSearch;
    /** The view takes the keyboard when it is drawn. */
    focus?: boolean;
  } = $props();

  const Preview = $derived(session.kind === undefined ? undefined : PREVIEWS[session.kind]);
</script>

<div class="body">
  {#if session.status === 'failed'}
    <p class="message" role="alert">{displayName(session.error ?? '')}</p>
  {:else if session.status === 'opening'}
    <p class="message" role="status">Opening…</p>
  {:else if session.shown === 'text'}
    <TextView {session} hit={search?.hit} {focus} />
  {:else if session.shown === 'hex'}
    <HexView {session} hit={search?.hit} {focus} />
  {:else if Preview}
    <!-- A kind's own view: registered in previews.ts. -->
    <Preview {session} {focus} />
  {:else}
    <p class="message" data-kind={session.kind}>…</p>
  {/if}
</div>

<style>
  .body {
    position: relative;
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
  }

  .message {
    margin: 0;
    padding: 28px 16px;
    color: var(--dim);
    text-align: center;
  }

  .message[role='alert'] {
    color: var(--danger);
  }
</style>
