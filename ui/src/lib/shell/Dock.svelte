<script lang="ts">
  import type { Snippet } from 'svelte';

  import { RESULTS_NOTICE } from '$lib/ops/targets';

  import type { CommandLine } from './command-line.svelte';
  import CommandOutput from './CommandOutput.svelte';
  import type { Action } from './shortcuts';

  /**
   * The function keys, in key order. Delete is the one destructive key and is marked as such. `needsFolder`: the key
   * does nothing on search results (they are not a folder), which the dock says. F4 included: a result is edited
   * only after its folder has been opened. `action`: what the key does (its default in the keymap).
   */
  const KEYS: readonly { key: string; action: Action; label: string; hot?: boolean; needsFolder?: boolean }[] = [
    { key: 'F3', action: 'view', label: 'View' },
    { key: 'F4', action: 'edit', label: 'Edit', needsFolder: true },
    { key: 'F5', action: 'copy', label: 'Copy' },
    { key: 'F6', action: 'move', label: 'Move', needsFolder: true },
    { key: 'F7', action: 'newFolder', label: 'New folder', needsFolder: true },
    { key: 'F8', action: 'trash', label: 'Delete', hot: true },
  ];

  /** What F5 is called inside an archive: the selection is extracted. */
  const labelOf = (key: string, label: string) => (key === 'F5' && inArchive ? 'Extract' : label);

  let {
    inArchive = false,
    inResults = false,
    commandLine,
    onAction,
    children,
  }: {
    /** The active pane shows a folder inside an archive: there F5 extracts the selection instead of copying it. */
    inArchive?: boolean;
    /** The active pane shows search results: moving and making a folder are not possible there (the key says why when used). */
    inResults?: boolean;
    /** The command line: a line typed runs there, its output shows above the dock. */
    commandLine?: CommandLine;
    /** A function key was clicked: the action it stands for (F5 copy, also when it is called Extract). */
    onAction?: (action: Action) => void;
    /** Between the command line and the keys. */
    children?: Snippet;
  } = $props();

  let typed = $state('');

  async function onLineKey(event: KeyboardEvent) {
    if (commandLine === undefined || event.isComposing) {
      return;
    }

    if (event.key === 'Enter') {
      event.preventDefault();
      if (await commandLine.run(typed)) {
        typed = '';
      }
    } else if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
      event.preventDefault();
      const line = commandLine.recall(event.key === 'ArrowUp' ? -1 : 1);
      if (line !== undefined) {
        typed = line;
      }
    } else if (event.key === 'Escape' && commandLine.shown && !commandLine.running) {
      event.preventDefault();
      commandLine.close();
    }
  }
</script>

<!-- One row: the shell command line on the left, content-width function keys on the right, no separators; whatever
  the dock is given (the transfers chip) sits between them. -->
<footer class="dock">
  {#if commandLine}
    <CommandOutput {commandLine} />
  {/if}
  <label class="shell">
    <span class="prompt" aria-hidden="true">❯</span>
    <input
      placeholder="Shell command in the active folder…"
      spellcheck="false"
      autocomplete="off"
      aria-label="Shell command"
      bind:value={typed}
      onkeydown={onLineKey}
    />
  </label>
  {@render children?.()}
  <div class="keys">
    {#each KEYS as { key, action, label: plain, hot, needsFolder } (key)}
      {@const label = labelOf(key, plain)}
      {@const unavailable = inResults && needsFolder}
      <button
        class="dock-key fk"
        class:hot
        class:unavailable
        type="button"
        aria-label="{label} ({key})"
        aria-disabled={unavailable || undefined}
        title={unavailable ? RESULTS_NOTICE : undefined}
        onclick={() => onAction?.(action)}
      >
        <kbd>{key}</kbd><span class="label">{label}</span>
      </button>
    {/each}
  </div>
</footer>

<style>
  .dock {
    position: relative;
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    height: 44px;
    padding: 0 8px 0 12px;
    border-top: 1px solid var(--line);
    background: var(--panel);
  }

  .shell {
    flex: 1;
    min-width: 160px;
    height: 30px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px;
    border: 1px solid var(--line);
    border-radius: 7px;
    background: var(--bg);
  }

  .shell:focus-within {
    border-color: var(--accent);
  }

  .prompt {
    font: 12.5px var(--font-mono);
    color: var(--accent-2);
  }

  input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: none;
    color: var(--text);
    font: 12.5px var(--font-mono);
  }

  .keys {
    flex: none;
    display: flex;
    align-items: center;
    gap: 2px;
  }

  kbd {
    min-width: 24px;
    border-color: color-mix(in srgb, var(--accent) 30%, transparent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent-2);
    text-align: center;
  }

  /* Still clickable: the click says why it does nothing. */
  .fk.unavailable {
    opacity: 0.45;
  }

  .fk.hot {
    color: color-mix(in srgb, var(--danger) 80%, var(--text));
  }

  .fk.hot kbd {
    border-color: color-mix(in srgb, var(--danger) 30%, transparent);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
    color: var(--danger);
  }

  @media (max-width: 1000px) {
    .fk {
      padding: 0 6px;
    }

    .label {
      display: none;
    }
  }

  @media (max-width: 760px) {
    .dock {
      justify-content: center;
    }

    .shell {
      display: none;
    }
  }
</style>
