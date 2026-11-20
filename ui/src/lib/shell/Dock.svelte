<script lang="ts">
  /** The function keys, in key order. Delete is the one destructive key and is marked as such. */
  const KEYS: readonly { key: string; label: string; hot?: boolean }[] = [
    { key: 'F3', label: 'View' },
    { key: 'F4', label: 'Edit' },
    { key: 'F5', label: 'Copy' },
    { key: 'F6', label: 'Move' },
    { key: 'F7', label: 'New folder' },
    { key: 'F8', label: 'Delete', hot: true },
  ];
</script>

<!-- One row: the shell command line on the left, content-width function keys on the right, no separators. -->
<footer class="dock">
  <label class="shell">
    <span class="prompt" aria-hidden="true">❯</span>
    <input placeholder="Shell command in the active folder…" spellcheck="false" aria-label="Shell command" />
  </label>
  <div class="keys">
    {#each KEYS as { key, label, hot } (key)}
      <button class="fk" class:hot type="button" aria-label="{label} ({key})">
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

  .fk {
    height: 30px;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0 10px 0 6px;
    border: 1px solid transparent;
    border-radius: 7px;
    background: none;
    color: var(--text);
    white-space: nowrap;
  }

  .fk:hover {
    border-color: var(--line);
    background: var(--panel-2);
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
