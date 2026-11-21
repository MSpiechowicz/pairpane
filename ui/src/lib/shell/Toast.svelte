<script lang="ts">
  let {
    text,
    error = false,
    undoable = false,
    onUndo,
    onClose,
  }: {
    text: string;
    /** A failure: shown in the danger colour. */
    error?: boolean;
    /** Offers Undo, which does not take the focus until it is used. */
    undoable?: boolean;
    onUndo: () => void;
    onClose: () => void;
  } = $props();

  /** How long the card stays; hovering or focusing it stops the clock. */
  const LIFETIME = 8000;

  let hovered = $state(false);
  let focused = $state(false);
  let left = LIFETIME;
  const paused = $derived(hovered || focused);

  $effect(() => {
    if (paused) {
      return;
    }

    const started = Date.now();
    const timer = setTimeout(() => onClose(), left);

    return () => {
      clearTimeout(timer);
      left -= Date.now() - started;
    };
  });
</script>

<section
  class="fade-in w-80 rounded-lg border border-line bg-panel p-4 text-sm text-fg shadow-lg"
  aria-live="polite"
  aria-label="Notice"
  onmouseenter={() => (hovered = true)}
  onmouseleave={() => (hovered = false)}
  onfocusin={() => (focused = true)}
  onfocusout={(event) => (focused = event.currentTarget.contains(event.relatedTarget as Node | null))}
>
  <p class="m-0" class:text-danger={error}>{text}</p>
  <div class="mt-3 flex justify-end gap-2">
    {#if undoable}
      <button class="btn" type="button" onclick={onUndo}>Undo</button>
    {/if}
    <button class="btn" type="button" onclick={onClose}>Close</button>
  </div>
</section>
