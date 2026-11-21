import type { Answer } from './bindings/Answer';
import type { AppInfo } from './bindings/AppInfo';
import type { ArchiveListing } from './bindings/ArchiveListing';
import type { ControlAction } from './bindings/ControlAction';
import type { ItemProperties } from './bindings/ItemProperties';
import type { JobId } from './bindings/JobId';
import type { JobTask } from './bindings/JobTask';
import type { LinkKind } from './bindings/LinkKind';
import type { ListEvent } from './bindings/ListEvent';
import type { MenuCommand } from './bindings/MenuCommand';
import type { MdDocument } from './bindings/MdDocument';
import type { OpKind } from './bindings/OpKind';
import type { OpsEvent } from './bindings/OpsEvent';
import type { PaneId } from './bindings/PaneId';
import type { SavedState } from './bindings/SavedState';
import type { SearchHit } from './bindings/SearchHit';
import type { SearchQuery } from './bindings/SearchQuery';
import type { SizeEvent } from './bindings/SizeEvent';
import type { TextQuery } from './bindings/TextQuery';
import type { TextWindow } from './bindings/TextWindow';
import type { TrashOutcome } from './bindings/TrashOutcome';
import type { UpdateInfo } from './bindings/UpdateInfo';
import type { UpdateProgress } from './bindings/UpdateProgress';
import type { ViewOpened } from './bindings/ViewOpened';
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
  ops_subscribe: cmd<{ onEvent: EventSink<OpsEvent> }, null>(),
  ops_start: cmd<{ kind: OpKind; from: string; names: string[]; to: string; verify: boolean }, JobId>(),
  ops_task: cmd<{ from: string; names: string[]; task: JobTask }, JobId>(),
  ops_control: cmd<{ job: JobId; action: ControlAction }, null>(),
  ops_answer: cmd<{ job: JobId; question: number; answer: Answer; all: boolean }, null>(),
  ops_rename: cmd<{ path: string; from: string; to: string }, null>(),
  ops_trash: cmd<{ path: string; names: string[] }, TrashOutcome>(),
  ops_untrash: cmd<{ token: number }, number>(),
  ops_delete: cmd<{ path: string; names: string[] }, null>(),
  ops_mkdir: cmd<{ path: string; name: string }, string>(),
  ops_mkfile: cmd<{ path: string; name: string }, boolean>(),
  ops_properties: cmd<{ path: string; name: string }, ItemProperties>(),
  ops_link: cmd<{ kind: LinkKind; path: string; name: string; to: string; linkName: string }, null>(),
  view_open: cmd<{ path: string; name: string }, ViewOpened>(),
  view_close: cmd<{ session: number }, null>(),
  view_bytes: cmd<{ session: number; offset: number; length: number }, ArrayBuffer>(),
  view_image_decode: cmd<{ session: number }, ArrayBuffer>(),
  view_text: cmd<{ session: number; query: TextQuery }, TextWindow>(),
  view_search: cmd<{ session: number; query: SearchQuery; onProgress: EventSink<number> }, SearchHit | null>(),
  view_search_cancel: cmd<{ session: number }, null>(),
  view_markdown: cmd<{ session: number; encoding: string }, MdDocument>(),
  view_archive: cmd<{ session: number }, ArchiveListing>(),
  view_media: cmd<{ session: number }, string>(),
  state_load: cmd<undefined, SavedState | null>(),
  state_save: cmd<{ state: SavedState }, null>(),
};

export type CommandName = keyof typeof commands;
export type CommandArgs<K extends CommandName> = (typeof commands)[K]['args'];
export type CommandResult<K extends CommandName> = (typeof commands)[K]['result'];
