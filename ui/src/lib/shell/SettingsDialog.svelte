<script lang="ts">
  import type { Preferences as Saved } from '$lib/ipc/bindings/Preferences';
  import type { ThemeChoice } from '$lib/ipc/bindings/ThemeChoice';
  import '$lib/ops/fields.css';

  import Modal from './Modal.svelte';
  import { type Preferences } from './preferences.svelte';

  let {
    preferences,
    onEditSettings,
    onClose,
  }: {
    preferences: Preferences;
    /** "Edit settings file…": the settings file opens in the system's editor for it. */
    onEditSettings: () => void;
    onClose: () => void;
  } = $props();

  /** The preferences that are an on or an off (the theme is the one that is not). */
  type Flag = Exclude<keyof Saved, 'theme'>;

  interface Toggle {
    flag: Flag;
    text: string;
  }

  const THEMES: readonly { choice: ThemeChoice; text: string }[] = [
    { choice: 'system', text: 'System' },
    { choice: 'light', text: 'Light' },
    { choice: 'dark', text: 'Dark' },
  ];

  const COLUMNS: readonly Toggle[] = [
    { flag: 'showExt', text: 'Ext' },
    { flag: 'showSize', text: 'Size' },
    { flag: 'showModified', text: 'Modified' },
  ];

  const FILES: readonly Toggle[] = [{ flag: 'showHidden', text: 'Show hidden files' }];

  const CONFIRMATIONS: readonly Toggle[] = [
    { flag: 'confirmTrash', text: 'Ask before moving to the Trash' },
    { flag: 'confirmTransfer', text: 'Ask for the target folder when copying or moving' },
    { flag: 'confirmQuitWithTransfers', text: 'Ask before quitting while transfers are running' },
  ];

  const UPDATES: readonly Toggle[] = [{ flag: 'checkUpdatesAtStart', text: 'Check for updates at start' }];
</script>

{#snippet toggles(items: readonly Toggle[])}
  {#each items as { flag, text } (flag)}
    <label class="check">
      <input type="checkbox" checked={preferences.value[flag]} onchange={(event) => preferences.set({ [flag]: event.currentTarget.checked })} />{text}
    </label>
  {/each}
{/snippet}

<Modal title="Settings" {onClose}>
  <div class="sections">
    <fieldset class="group">
      <legend>Appearance</legend>
      {#each THEMES as { choice, text } (choice)}
        <label class="check">
          <input type="radio" name="theme" checked={preferences.value.theme === choice} onchange={() => preferences.set({ theme: choice })} />{text}
        </label>
      {/each}
    </fieldset>

    <fieldset class="group">
      <legend>Columns</legend>
      {@render toggles(COLUMNS)}
    </fieldset>

    <fieldset class="group">
      <legend>Files</legend>
      {@render toggles(FILES)}
    </fieldset>

    <fieldset class="group">
      <legend>Confirmations</legend>
      {@render toggles(CONFIRMATIONS)}
      <p class="note">Deleting for good always asks.</p>
    </fieldset>

    <fieldset class="group">
      <legend>Editor</legend>
      <p class="note">The external editor for F4 is set in the settings file.</p>
      <button class="btn" type="button" onclick={onEditSettings}>Edit settings file…</button>
    </fieldset>

    <fieldset class="group">
      <legend>Updates</legend>
      {@render toggles(UPDATES)}
    </fieldset>
  </div>

  <div class="footer">
    <button class="btn btn-primary" type="button" data-autofocus onclick={onClose}>Close</button>
  </div>
</Modal>

<style>
  .sections {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-height: min(520px, 62vh);
    overflow: auto;
  }

  .note {
    flex-basis: 100%;
    margin: 0;
    color: var(--dim);
  }

  .footer {
    display: flex;
    justify-content: flex-end;
    margin-top: 12px;
  }
</style>
