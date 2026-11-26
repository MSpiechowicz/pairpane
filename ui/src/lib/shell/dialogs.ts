import type { CommandId } from '$lib/palette/commands';

/** The dialogs of the top bar's Settings menu. */
export type ShellDialog = 'keys' | 'settings' | 'devices';

/** The palette commands that open one of them. */
export const DIALOG_OF = {
  keyboardShortcuts: 'keys',
  settings: 'settings',
  devices: 'devices',
} as const satisfies Partial<Record<CommandId, ShellDialog>>;

/** Whether the palette command `id` opens a dialog of the Settings menu. */
export function opensDialog(id: CommandId): id is keyof typeof DIALOG_OF {
  return Object.hasOwn(DIALOG_OF, id);
}
