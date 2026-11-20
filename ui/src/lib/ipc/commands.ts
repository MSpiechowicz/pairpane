import type { AppInfo } from './bindings/AppInfo';
import { cmd } from './command-spec';

/** Every backend command with its argument and result types; the key is the Rust `#[tauri::command]` name. */
export const commands = {
  app_info: cmd<undefined, AppInfo>(),
};

export type CommandName = keyof typeof commands;
export type CommandArgs<K extends CommandName> = (typeof commands)[K]['args'];
export type CommandResult<K extends CommandName> = (typeof commands)[K]['result'];
