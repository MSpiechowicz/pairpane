/** Gives the focus to the active pane's list, unless it is somewhere already (a dialog, a field, a button). */
export function focusActiveList() {
  if (document.activeElement === null || document.activeElement === document.body) {
    document.querySelector<HTMLElement>('[data-pane][aria-current="true"] [role="listbox"]')?.focus();
  }
}
