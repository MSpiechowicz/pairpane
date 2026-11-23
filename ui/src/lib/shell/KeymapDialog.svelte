<script lang="ts">
  import { COMMANDS } from '$lib/palette/commands';
  import { displayName } from '$lib/panes/format';

  import { type Keymap } from './keymap.svelte';
  import Modal from './Modal.svelte';
  import { ACTIONS, type Action, chordOf, written } from './shortcuts';

  let { keymap, onClose }: { keymap: Keymap; onClose: () => void } = $props();

  /** The names the palette gives the actions, and the one it has none for (itself). */
  const LABELS = new Map<string, string>([...COMMANDS.map(({ id, label }) => [id, label] as const), ['palette', 'Command palette']]);

  const rows = ACTIONS.map((action) => ({ action, label: LABELS.get(action) ?? action })).sort((a, b) => a.label.localeCompare(b.label));

  /** The action whose new key is awaited. */
  let changing = $state<Action | undefined>();
  /** What the last change did, or why it was refused. */
  let notice = $state<{ text: string; error: boolean } | undefined>();

  const labelOf = (action: Action) => LABELS.get(action) ?? action;

  /** The key pressed while a change waits becomes the action's (Escape gives up). */
  function capture(event: KeyboardEvent) {
    if (changing === undefined) {
      return;
    }

    event.preventDefault();
    event.stopPropagation();
    if (event.key === 'Escape' && !event.ctrlKey && !event.altKey && !event.shiftKey) {
      changing = undefined;
      notice = undefined;
      return;
    }

    const chord = chordOf(event);
    if (chord === undefined) {
      return; // a modifier on its own: the key comes next
    }

    const action = changing;
    const done = keymap.set(action, chord);
    if (typeof done === 'string') {
      notice = { text: done, error: true };
      return;
    }

    changing = undefined;
    notice = {
      text:
        done.from === undefined
          ? `${labelOf(action)}: ${written(chord)}.`
          : `${labelOf(action)}: ${written(chord)}, taken from “${labelOf(done.from)}”.`,
      error: false,
    };
  }
</script>

<Modal title="Keyboard shortcuts" variant="wide" onClose={() => (changing === undefined ? onClose() : (changing = undefined))}>
  <div class="keys" role="table" aria-label="Shortcuts">
    {#each rows as { action, label } (action)}
      {@const keys = keymap.allKeysOf(action)}
      <div class="row" role="row">
        <span class="label" role="cell">{displayName(label)}</span>
        <span class="chords" role="cell">
          {#if changing === action}
            <em>Press the new keys… (Esc gives up)</em>
          {:else if keys.length === 0}
            <em>No key</em>
          {:else}
            {#each keys as chord (chord)}<kbd>{displayName(chord)}</kbd>{/each}
          {/if}
        </span>
        <span class="buttons" role="cell">
          {#if changing === action}
            <!-- The keys pressed next are the new shortcut: they go to this button. -->
            <button class="btn" type="button" {@attach (node) => node.focus()} onkeydown={capture} onclick={() => (changing = undefined)}>Cancel</button>
          {:else}
            <button class="btn" type="button" aria-label="Change the keys of {label}" onclick={() => ((changing = action), (notice = undefined))}>
              Change
            </button>
            <button class="btn" type="button" aria-label="Reset the keys of {label}" disabled={!keymap.isCustom(action)} onclick={() => keymap.reset(action)}>
              Reset
            </button>
          {/if}
        </span>
      </div>
    {/each}
  </div>

  {#if notice}
    <p class="notice" class:error={notice.error} role="status">{displayName(notice.text)}</p>
  {/if}

  <div class="footer">
    <button class="btn" type="button" onclick={() => ((changing = undefined), keymap.resetAll(), (notice = undefined))}>
      Reset all to the defaults
    </button>
    <button class="btn btn-primary" type="button" data-autofocus onclick={onClose}>Close</button>
  </div>
</Modal>

<style>
  .keys {
    max-height: min(460px, 60vh);
    overflow: auto;
  }

  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 180px) auto;
    gap: 10px;
    align-items: center;
    padding: 3px 4px;
    border-bottom: 1px solid var(--line);
  }

  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chords {
    display: flex;
    gap: 6px;
    color: var(--dim);
    font-size: 12px;
  }

  .buttons {
    display: flex;
    gap: 6px;
  }

  .notice {
    margin: 8px 0 0;
    color: var(--dim);
  }

  .notice.error {
    color: var(--danger);
  }

  .footer {
    display: flex;
    justify-content: space-between;
    margin-top: 12px;
  }
</style>
