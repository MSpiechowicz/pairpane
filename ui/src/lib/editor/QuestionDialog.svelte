<script lang="ts">
  import ChoiceDialog from '$lib/shell/ChoiceDialog.svelte';

  import type { Question } from './saveFlow.svelte';

  let { question }: { question: Question } = $props();
</script>

<ChoiceDialog
  title={question.title}
  message={question.message}
  confirm={question.confirm}
  alternate={question.alternate}
  danger={question.danger}
  onConfirm={() => question.answer('confirm')}
  onAlternate={() => question.answer('alternate')}
  onCancel={() => question.answer('cancel')}
/>
