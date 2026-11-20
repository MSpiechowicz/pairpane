import type { AppInfo } from './bindings/AppInfo';
import type { ListEvent } from './bindings/ListEvent';
import type { MenuCommand } from './bindings/MenuCommand';
import type { PaneId } from './bindings/PaneId';
import type { SavedState } from './bindings/SavedState';
import type { SizeEvent } from './bindings/SizeEvent';
import type { UpdateInfo } from './bindings/UpdateInfo';
import type { UpdateProgress } from './bindings/UpdateProgress';
import type { VolumeEvent } from './bindings/VolumeEvent';
import { cmd } from './command-spec';
import type { EventSink } from './transport';

/** Every backend command with its argument and result types; the key is the Rust `#[tauri::command]` name. */
export const commands = {
  app_info: cmd<undefined, AppInfo>(),
  update_check: cmd<undefined, UpdateInfo | null>(),
  update_install: cmd<{ version: string; onProgress: EventSink<UpdateProgress> }, null>(),
  app_relaunch: cmd<undefined, null>(),
  menu_subscribe: cmd<{ onCommand: EventSink<MenuCommand> }, null>(),
  pane_open: cmd<
    {
      pane: PaneId;
      path: string | null;
      child: string | null;
      showHidden: boolean;
      onEvent: EventSink<ListEvent>;
    },
    null
  >(),
  pane_folder_size: cmd<{ pane: PaneId; path: string; name: string; onEvent: EventSink<SizeEvent> }, null>(),
  volumes_list: cmd<{ onEvent: EventSink<VolumeEvent> }, null>(),
  state_load: cmd<undefined, SavedState | null>(),
  state_save: cmd<{ state: SavedState }, null>(),
};

export type CommandName = keyof typeof commands;
export type CommandArgs<K extends CommandName> = (typeof commands)[K]['args'];
export type CommandResult<K extends CommandName> = (typeof commands)[K]['result'];
