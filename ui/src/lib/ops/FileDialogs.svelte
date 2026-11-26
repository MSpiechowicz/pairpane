<script lang="ts">
  import type { Transport } from '$lib/ipc/transport';
  import type { SavedRenames } from '$lib/rename/saved.svelte';

  import ArchivePasswordDialog from './ArchivePasswordDialog.svelte';
  import ConflictDialog from './ConflictDialog.svelte';
  import DeleteDialog from './DeleteDialog.svelte';
  import type { FileActions } from './fileActions.svelte';
  import OpDialog from './OpDialog.svelte';
  import type { Passwords } from './passwords.svelte';
  import ToolDialogs from './ToolDialogs.svelte';
  import type { Transfers } from './transfers.svelte';

  let {
    ipc,
    files,
    transfers,
    passwords,
    renames,
  }: {
    ipc: Transport;
    /** The dialogs of the file keys, and what they answer to. */
    files: FileActions;
    /** The question of a job that found a conflict. */
    transfers: Transfers;
    /** The password question of an archive. */
    passwords: Passwords;
    renames: SavedRenames;
  } = $props();

  const question = $derived(transfers.question);
</script>

{#if files.dialog}
  <OpDialog
    request={files.dialog.request}
    {ipc}
    onClose={() => files.closeDialog()}
    onDone={(name) => files.dialogDone(name)}
    onNotice={(notice) => files.say(notice.text, notice.error)}
  />
{/if}

{#if files.tools.dialog}
  <ToolDialogs
    {ipc}
    dialog={files.tools.dialog}
    {renames}
    onClose={() => files.closeTool()}
    onDone={(output) => files.toolDone(output)}
  />
{/if}

{#if files.removal}
  <DeleteDialog
    permanent={files.removal.permanent}
    names={files.removal.names}
    onConfirm={() => files.confirmRemoval()}
    onClose={() => files.closeRemoval()}
  />
{/if}

{#if question}
  {#key `${question.info.job}:${question.info.question}`}
    <ConflictDialog {transfers} {question} />
  {/key}
{/if}

{#if passwords.prompt}
  {#key passwords.prompt.archive}
    <ArchivePasswordDialog {passwords} prompt={passwords.prompt} />
  {/key}
{/if}
