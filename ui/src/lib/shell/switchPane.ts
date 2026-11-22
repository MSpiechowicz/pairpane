import { isTyping } from './shortcuts';

/**
 * Whether a key press switches the active pane: Tab while focus is in a pane's list or nowhere (the page itself). In
 * the top bar, the menus (a pane's dropdowns too), the dock and a pane's text fields (the mask prompt) it moves focus
 * as usual, so every control stays reachable from the keyboard. While the quick view is open there is one pane to work in.
 */
export function switchesPane(event: KeyboardEvent, quick: boolean): boolean {
  const target = event.target;
  const inPane = target === document.body || (target instanceof Element && target.closest('[data-pane]'));
  const inMenu = target instanceof Element && target.closest('[data-menu]') !== null;

  return !(
    quick ||
    event.key !== 'Tab' ||
    event.shiftKey ||
    event.ctrlKey ||
    event.altKey ||
    event.metaKey ||
    !inPane ||
    inMenu ||
    isTyping(target)
  );
}
