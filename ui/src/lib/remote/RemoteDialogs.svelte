<script lang="ts">
  import { displayName } from '$lib/panes/format';
  import PasswordDialog from '$lib/ops/PasswordDialog.svelte';

  import ConnectDialog from './ConnectDialog.svelte';
  import type { Remote } from './remote.svelte';

  let { remote }: { remote: Remote } = $props();

  const prompt = $derived(remote.prompt);
  const host = $derived(displayName(remote.target?.host ?? ''));
  const passphrase = $derived(prompt?.secret === 'passphrase');
  const what = $derived(passphrase ? 'passphrase' : 'password');
  const message = $derived(
    prompt?.wrong
      ? `The ${what} is wrong. Try again.`
      : passphrase
        ? 'The private key is protected by a passphrase.'
        : 'The server asks for a password.',
  );
  // Told apart from the question: the key opened, but the server turned it down, so it asks for something else.
  const notice = $derived(prompt?.reason === 'keyNotAccepted' ? 'The key opened, but the server did not accept it.' : undefined);
  const alternate = $derived(
    prompt?.other === 'password' ? 'Use password instead' : prompt?.other === 'passphrase' ? "Use the key's passphrase instead" : undefined,
  );
</script>

{#if remote.shown}
  <ConnectDialog {remote} />
{/if}

{#if prompt}
  <!-- A new dialog whenever the asked kind changes (switched, or turned over by the server): nothing typed is carried across. -->
  {#key prompt.secret}
    <PasswordDialog
      title="{passphrase ? 'Passphrase' : 'Password'} for “{host}”"
      {message}
      {notice}
      alert={prompt.wrong}
      label={passphrase ? 'Passphrase' : 'Password'}
      failure={prompt.failure}
      busy={prompt.busy}
      {alternate}
      onAlternate={() => remote.switchSecret()}
      onSubmit={(secret) => remote.submitSecret(secret)}
      onCancel={() => remote.cancelSecret()}
    />
  {/key}
{/if}
