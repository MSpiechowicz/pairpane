<script lang="ts">
  import type { Transport } from '$lib/ipc/transport';
  import type { QuitGuard } from '$lib/ops/quit.svelte';

  import ChoiceDialog from './ChoiceDialog.svelte';
  import { keymap } from './keymap.svelte';
  import KeymapDialog from './KeymapDialog.svelte';
  import { preferences } from './preferences.svelte';
  import SettingsDialog from './SettingsDialog.svelte';

  let {
    open,
    ipc,
    quit,
    onEditSettings,
    onClose,
  }: {
    /** The dialog of the top bar's Settings menu that is open. */
    open: 'keys' | 'settings' | undefined;
    ipc: Transport;
    /** Its question about quitting with transfers running, when it has one on the screen. */
    quit: QuitGuard;
    /** "Edit settings file…" of the settings dialog. */
    onEditSettings: () => void;
    onClose: () => void;
  } = $props();
</script>

{#if open === 'keys'}
  <KeymapDialog {keymap} {onClose} />
{:else if open === 'settings'}
  <SettingsDialog {preferences} loadEditor={() => ipc.invoke('settings_editor', undefined)} onEditSettings={() => (onClose(), onEditSettings())} {onClose} />
{/if}

{#if quit.asking}
  <ChoiceDialog
    title="Quit Pairpane?"
    message="Transfers are still running. Quit anyway?"
    confirm="Quit"
    danger
    onConfirm={() => quit.answer(true)}
    onCancel={() => quit.answer(false)}
  />
{/if}
