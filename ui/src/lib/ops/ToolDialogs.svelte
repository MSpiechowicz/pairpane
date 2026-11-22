<script lang="ts">
  import type { Transport } from '$lib/ipc/transport';
  import RenameDialog from '$lib/rename/RenameDialog.svelte';
  import type { SavedRenames } from '$lib/rename/saved.svelte';

  import AttributesDialog from './AttributesDialog.svelte';
  import ChecksumDialog from './ChecksumDialog.svelte';
  import PackDialog from './PackDialog.svelte';
  import PropertiesDialog from './PropertiesDialog.svelte';
  import SplitDialog from './SplitDialog.svelte';
  import type { ToolDialog } from './tools.svelte';

  let {
    ipc,
    dialog,
    renames,
    onClose,
    onDone,
  }: {
    ipc: Transport;
    dialog: ToolDialog;
    /** The rename rules kept by name. */
    renames: SavedRenames;
    onClose: () => void;
    /** A job started; a checksum file says its name. */
    onDone: (output?: string) => void;
  } = $props();
</script>

{#if dialog.kind === 'properties'}
  <PropertiesDialog {ipc} path={dialog.path} entry={dialog.entry} sizes={dialog.source.sizes} {onClose} />
{:else if dialog.kind === 'attributes'}
  <AttributesDialog {ipc} path={dialog.path} entries={dialog.entries} {onClose} {onDone} />
{:else if dialog.kind === 'checksum'}
  <ChecksumDialog {ipc} path={dialog.path} names={dialog.names} {onClose} {onDone} />
{:else if dialog.kind === 'split'}
  <SplitDialog {ipc} path={dialog.path} entry={dialog.entry} to={dialog.to} {onClose} {onDone} />
{:else if dialog.kind === 'pack'}
  <PackDialog {ipc} path={dialog.path} names={dialog.names} to={dialog.to} {onClose} {onDone} />
{:else}
  <RenameDialog {ipc} path={dialog.path} names={dialog.names} {renames} {onClose} {onDone} />
{/if}
