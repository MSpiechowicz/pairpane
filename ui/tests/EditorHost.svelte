<script lang="ts">
  // Test helper: the editor's overlay as the window shows it.
  import type { Editor } from '$lib/editor/editor.svelte';
  import EditorOverlay from '$lib/editor/EditorOverlay.svelte';

  let { editor }: { editor: Editor } = $props();
</script>

{#if editor.session}
  <EditorOverlay {editor} session={editor.session} />
{/if}
