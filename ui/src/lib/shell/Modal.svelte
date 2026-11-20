<script lang="ts" module>
  /** The dialogs that are open, the last one on top: only that one answers the keyboard. */
  const stack: symbol[] = [];

  const FOCUSABLE = 'button:not(:disabled), input:not(:disabled), [tabindex]:not([tabindex="-1"])';
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    title,
    onClose,
    children,
  }: {
    title: string;
    /** Escape. Without it the dialog can only be left by one of its own buttons. */
    onClose?: () => void;
    children: Snippet;
  } = $props();

  const id = Symbol('modal');
  const titleId = $props.id();
  let dialog = $state<HTMLElement>();
  // Read while the dialog is being made: its content may already take the focus when it is mounted.
  const before = document.activeElement;

  /** Takes the focus in (the field or button marked `data-autofocus`, else the first one) and gives it back on close. */
  function wire(node: HTMLElement) {
    stack.push(id);
    const first = node.querySelector<HTMLElement>('[data-autofocus]') ?? node.querySelector<HTMLElement>(FOCUSABLE);
    (first ?? node).focus();

    return () => {
      stack.splice(stack.indexOf(id), 1);
      if (before instanceof HTMLElement && before.isConnected) {
        before.focus();
      }
    };
  }

  /** Tab goes round inside the dialog; a focus that got out (a click on the backdrop) comes back in. */
  function trap(event: KeyboardEvent, node: HTMLElement) {
    const stops = [...node.querySelectorAll<HTMLElement>(FOCUSABLE)];
    const [first, last] = [stops[0], stops.at(-1)];
    const inside = event.target instanceof Node && node.contains(event.target);

    if (!inside || stops.length === 0) {
      event.preventDefault();
      (first ?? node).focus();
    } else if (event.shiftKey && event.target === first) {
      event.preventDefault();
      last?.focus();
    } else if (!event.shiftKey && event.target === last) {
      event.preventDefault();
      first?.focus();
    }
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.isComposing || stack.at(-1) !== id || !dialog) {
      return;
    }

    if (event.key === 'Escape' && onClose) {
      event.preventDefault();
      onClose();
    } else if (event.key === 'Tab') {
      trap(event, dialog);
    }
  }
</script>

<svelte:window {onkeydown} />

<div class="backdrop fade-in">
  <div class="popup modal" role="dialog" aria-modal="true" aria-labelledby={titleId} tabindex="-1" bind:this={dialog} {@attach wire}>
    <h2 id={titleId}>{title}</h2>
    {@render children()}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: 18vh;
    background: rgb(0 0 0 / 0.45);
  }

  .modal {
    width: min(520px, calc(100vw - 32px));
    padding: 16px;
  }

  .modal:focus {
    outline: none;
  }

  h2 {
    margin: 0 0 12px;
    font-size: 13px;
    font-weight: 600;
  }
</style>
