<script lang="ts">
  import type { Snippet } from 'svelte';

  /** The function keys, in key order. Delete is the one destructive key and is marked as such. */
  const KEYS: readonly { key: string; label: string; hot?: boolean }[] = [
    { key: 'F3', label: 'View' },
    { key: 'F4', label: 'Edit' },
    { key: 'F5', label: 'Copy' },
    { key: 'F6', label: 'Move' },
    { key: 'F7', label: 'New folder' },
    { key: 'F8', label: 'Delete', hot: true },
  ];

  /** What F5 is called inside an archive: the selection is extracted. */
  const labelOf = (key: string, label: string) => (key === 'F5' && inArchive ? 'Extract' : label);

  let {
    inArchive = false,
    onKey,
    children,
  }: {
    /** The active pane shows a folder inside an archive: there F5 extracts the selection instead of copying it. */
    inArchive?: boolean;
    /** A function key was clicked; it does what pressing it does. */
    onKey?: (key: string) => void;
    /** Between the command line and the keys. */
    children?: Snippet;
  } = $props();
</script>

<!-- One row: the shell command line on the left, content-width function keys on the right, no separators; whatever
  the dock is given (the transfers chip) sits between them. -->
<footer class="dock">
  <label class="shell">
    <span class="prompt" aria-hidden="true">❯</span>
    <input placeholder="Shell command in the active folder…" spellcheck="false" aria-label="Shell command" />
  </label>
  {@render children?.()}
  <div class="keys">
    {#each KEYS as { key, label: plain, hot } (key)}
      {@const label = labelOf(key, plain)}
      <button class="dock-key fk" class:hot type="button" aria-label="{label} ({key})" onclick={() => onKey?.(key)}>
        <kbd>{key}</kbd><span class="label">{label}</span>
      </button>
    {/each}
  </div>
</footer>

<style>
  .dock {
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
