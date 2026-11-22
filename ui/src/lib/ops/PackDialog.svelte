<script lang="ts">
  import { untrack } from 'svelte';

  import type { PackFormat } from '$lib/ipc/bindings/PackFormat';
  import type { PackLevel } from '$lib/ipc/bindings/PackLevel';
  import type { Transport } from '$lib/ipc/transport';

  import { what } from './dialogs';
  import JobDialog from './JobDialog.svelte';
  import { canEncrypt, levelOr, levelsFor, PACK_FORMATS, packPath, rememberedPack, rememberPack, swapExtension } from './pack';
  import { Submission } from './submission.svelte';
  import './fields.css';

  let {
    ipc,
    path,
    names,
    to,
    onClose,
    onDone,
  }: {
    ipc: Transport;
    /** The folder the items are in. */
    path: string;
    names: string[];
    /** The folder the archive is made in to begin with: the other pane's. */
    to: string;
    onClose: () => void;
    /** The job started. */
    onDone: () => void;
  } = $props();

  const start = untrack(rememberedPack);

  let format = $state<PackFormat>(start.format);
  let level = $state<PackLevel>(start.level);
  let encryptNames = $state(start.encryptNames);
  let archive = $state(untrack(() => packPath(to, names, path, start.format)));
  // What is typed lives only here, and only until it is sent.
  let password = $state('');
  let confirmation = $state('');
  const submission = new Submission(() => onClose());

  const sealed = $derived(canEncrypt(format));
  const mismatch = $derived(password !== confirmation);

  function chooseFormat(next: PackFormat) {
    format = next;
    level = levelOr(next, level);
    archive = swapExtension(archive, next);

    if (!canEncrypt(next)) {
      password = '';
      confirmation = '';
    }
  }

  async function submit() {
    if (submission.busy || archive.trim() === '') {
      return;
    }

    if (sealed && mismatch) {
      submission.refuse('The passwords do not match.');
      return;
    }

    const secret = sealed && password !== '' ? password : null;
    rememberPack({ format, level, encryptNames });

    await submission.run(async () => {
      try {
        await ipc.invoke('ops_pack', {
          from: path,
          names,
          archive,
          format,
          level,
          password: secret,
          encryptNames: format === 'sevenZ' && secret !== null && encryptNames,
        });
      } finally {
        password = '';
        confirmation = '';
      }

      onDone();
    });
  }
</script>

<JobDialog title="Pack {what(names)}" confirm="Pack" {submission} onSubmit={submit}>
  <input
    class="field"
    type="text"
    spellcheck="false"
    autocomplete="off"
    aria-label="Archive"
    data-autofocus
    bind:value={archive}
  />
  <div class="row">
    <select class="field" aria-label="Format" value={format} onchange={(event) => chooseFormat(event.currentTarget.value as PackFormat)}>
      {#each PACK_FORMATS as { format: value, label } (value)}
        <option {value}>{label}</option>
      {/each}
    </select>
    <select class="field" aria-label="Level" value={level} onchange={(event) => (level = event.currentTarget.value as PackLevel)}>
      {#each levelsFor(format) as { level: value, label } (value)}
        <option {value}>{label}</option>
      {/each}
    </select>
  </div>
  <input
    class="field"
    type="password"
    spellcheck="false"
    autocomplete="off"
    placeholder="Password (optional)"
    aria-label="Password"
    disabled={!sealed}
    bind:value={password}
  />
  <input
    class="field"
    type="password"
    spellcheck="false"
    autocomplete="off"
    placeholder="Confirm password"
    aria-label="Confirm password"
    disabled={!sealed}
    bind:value={confirmation}
  />
  {#if format === 'sevenZ'}
    <label class="check"><input type="checkbox" bind:checked={encryptNames} disabled={password === ''} />Encrypt file names</label>
  {/if}
  {#if !sealed}
    <p class="hint">A tar.zst cannot have a password.</p>
  {:else if mismatch}
    <p class="hint">The passwords do not match.</p>
  {/if}
</JobDialog>

<style>
  .row {
    display: flex;
    gap: 8px;
  }

  .row > select {
    flex: 1;
    min-width: 0;
  }
</style>
