<script lang="ts">
  import DialogFooter from '$lib/ops/DialogFooter.svelte';
  import Modal from '$lib/shell/Modal.svelte';

  import type { Remote } from './remote.svelte';
  import { toTarget } from './target';
  import '$lib/ops/fields.css';

  let { remote }: { remote: Remote } = $props();

  let host = $state('');
  let user = $state('');
  let port = $state('');
  let keyFile = $state('');
  /** Why the fields cannot be sent; the server's own refusal is `remote.failure`. */
  let problem = $state<string | undefined>();

  const uid = $props.id();
  const listId = `${uid}-hosts`;
  /** The host of the SSH configuration the field names, whose user and port the empty fields stand for. */
  const named = $derived(remote.hosts.find((candidate) => candidate.alias === host.trim()));

  async function submit() {
    const target = toTarget({ host, user, port, keyFile });
    problem = typeof target === 'string' ? target : undefined;

    if (typeof target !== 'string') {
      await remote.connect(target);
    }
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.isComposing) {
      event.preventDefault();
      void submit();
    }
  }
</script>

<Modal title="Connect to SFTP server" onClose={() => remote.close()}>
  <!-- Enter in a field connects; the stack of fields is one column. -->
  <div class="fields" role="presentation" {onkeydown}>
    <label class="labelled">
      Host
      <input
        class="field"
        type="text"
        spellcheck="false"
        autocomplete="off"
        list={listId}
        placeholder="example.com, or a host of ~/.ssh/config"
        data-autofocus
        bind:value={host}
      />
    </label>
    <datalist id={listId}>
      {#each remote.hosts as { alias, hostName } (alias)}
        <option value={alias}>{hostName ?? ''}</option>
      {/each}
    </datalist>
    <label class="labelled">
      User
      <input class="field" type="text" spellcheck="false" autocomplete="off" placeholder={named?.user ?? 'your user name'} bind:value={user} />
    </label>
    <label class="labelled">
      Port
      <input class="field" type="text" inputmode="numeric" autocomplete="off" placeholder={String(named?.port ?? 22)} bind:value={port} />
    </label>
    <label class="labelled">
      Key file
      <input class="field" type="text" spellcheck="false" autocomplete="off" placeholder="optional: a private key to try first" bind:value={keyFile} />
    </label>
  </div>
  <p class="hint">The password or passphrase, if one is needed, is asked for next. It is kept in memory only, until you quit.</p>
  <DialogFooter failure={problem ?? remote.failure} busy={remote.busy} confirm="Connect" onConfirm={submit} onCancel={() => remote.close()} />
</Modal>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .labelled {
    flex: none;
  }

  .labelled :global(.field) {
    min-width: 0;
  }
</style>
