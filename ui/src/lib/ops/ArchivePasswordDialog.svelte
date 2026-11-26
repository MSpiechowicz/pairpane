<script lang="ts">
  import { displayName } from '$lib/panes/format';

  import type { Passwords, Prompt } from './passwords.svelte';
  import PasswordDialog from './PasswordDialog.svelte';

  /** The question of an archive that needs a password, in the shared password dialog. */
  let { passwords, prompt }: { passwords: Passwords; prompt: Prompt } = $props();

  const wrong = $derived(prompt.reason === 'wrong');
</script>

<PasswordDialog
  title="Password for “{displayName(prompt.name)}”"
  message={wrong ? 'The password is wrong. Try again.' : 'This archive needs a password.'}
  alert={wrong}
  failure={prompt.failure}
  busy={prompt.busy}
  onSubmit={(password) => passwords.submit(password)}
  onCancel={() => passwords.cancel()}
/>
