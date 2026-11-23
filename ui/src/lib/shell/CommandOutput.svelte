<script lang="ts">
  import { displayName } from '$lib/panes/format';

  import type { CommandLine } from './command-line.svelte';

  let { commandLine }: { commandLine: CommandLine } = $props();

  const shown = $derived(commandLine.shown);

  /** What the status line says of how the command stands. */
  const status = $derived.by(() => {
    switch (shown?.status) {
      case 'starting':
        return 'Starting…';
      case 'running':
        return 'Running…';
      case 'stopped':
        return 'Stopped';
      case 'failed':
        return 'Not run';
      case 'done':
        return shown.code === null || shown.code === undefined ? 'Ended' : `Exit code ${shown.code}`;
      default:
        return '';
    }
  });

  /** Keeps the newest output in view while it comes, unless the user scrolled up to read. */
  function follow(node: HTMLElement) {
    $effect(() => {
      void shown?.output;
      const atEnd = node.scrollHeight - node.scrollTop - node.clientHeight < 24;
      if (atEnd || commandLine.running) {
        node.scrollTop = node.scrollHeight;
      }
    });
  }
</script>

{#if shown}
  <section class="output" aria-label="Command output">
    <header class="head">
      <span class="line" title={displayName(shown.line)}><span aria-hidden="true">❯ </span>{displayName(shown.line)}</span>
      <span class="status" class:bad={shown.status === 'failed' || (shown.status === 'done' && shown.code !== 0)} role="status">
        {status}
      </span>
      {#if commandLine.running}
        <button class="btn" type="button" onclick={() => commandLine.stop()}>Stop</button>
      {/if}
      <button class="btn" type="button" aria-label="Close the output" onclick={() => commandLine.close()}>Close</button>
    </header>
    <pre class="text" tabindex="-1" {@attach follow}>{displayName(shown.output)}</pre>
    {#if shown.note}
      <p class="note">{displayName(shown.note)}</p>
    {/if}
  </section>
{/if}

<style>
  .output {
    position: absolute;
    left: 8px;
    right: 8px;
    bottom: 48px;
    z-index: 20;
    display: flex;
    flex-direction: column;
    max-height: min(320px, 45vh);
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--panel);
    box-shadow: 0 8px 24px rgb(0 0 0 / 25%);
  }

  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 8px 6px 12px;
    border-bottom: 1px solid var(--line);
  }

  .line {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: 12.5px var(--font-mono);
  }

  .status {
    color: var(--dim);
    font-size: 12px;
  }

  .status.bad {
    color: var(--danger);
  }

  .text {
    flex: 1;
    min-height: 40px;
    margin: 0;
    padding: 8px 12px;
    overflow: auto;
    font: 12px/1.45 var(--font-mono);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .note {
    margin: 0;
    padding: 4px 12px 8px;
    color: var(--dim);
    font-size: 12px;
  }
</style>
