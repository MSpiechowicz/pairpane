import type { Answer } from './bindings/Answer';
import type { AppInfo } from './bindings/AppInfo';
import type { ArchiveListing } from './bindings/ArchiveListing';
import type { ByteDifference } from './bindings/ByteDifference';
import type { CompareEvent } from './bindings/CompareEvent';
import type { CompareId } from './bindings/CompareId';
import type { CompareOptions } from './bindings/CompareOptions';
import type { CompareRoot } from './bindings/CompareRoot';
import type { ControlAction } from './bindings/ControlAction';
import type { EditOpened } from './bindings/EditOpened';
import type { EditSaved } from './bindings/EditSaved';
import type { EditSaveOptions } from './bindings/EditSaveOptions';
import type { ExternalEdit } from './bindings/ExternalEdit';
import type { FindEvent } from './bindings/FindEvent';
import type { FindId } from './bindings/FindId';
import type { FindQuery } from './bindings/FindQuery';
import type { ItemProperties } from './bindings/ItemProperties';
import type { JobId } from './bindings/JobId';
import type { JobTask } from './bindings/JobTask';
import type { LinkKind } from './bindings/LinkKind';
import type { ListEvent } from './bindings/ListEvent';
import type { MdDocument } from './bindings/MdDocument';
import type { MenuCommand } from './bindings/MenuCommand';
import type { OpKind } from './bindings/OpKind';
import type { OpsEvent } from './bindings/OpsEvent';
import type { PackFormat } from './bindings/PackFormat';
import type { PackLevel } from './bindings/PackLevel';
import type { PaneId } from './bindings/PaneId';
import type { RenamePreview } from './bindings/RenamePreview';
import type { RenameRule } from './bindings/RenameRule';
import type { SavedState } from './bindings/SavedState';
import type { SearchHit } from './bindings/SearchHit';
import type { SearchQuery } from './bindings/SearchQuery';
import type { SizeEvent } from './bindings/SizeEvent';
import type { SyncItem } from './bindings/SyncItem';
import type { SyncRemoval } from './bindings/SyncRemoval';
import type { TextQuery } from './bindings/TextQuery';
import type { TextWindow } from './bindings/TextWindow';
import type { TrashOutcome } from './bindings/TrashOutcome';
import type { UpdateInfo } from './bindings/UpdateInfo';
import type { UpdateProgress } from './bindings/UpdateProgress';
import type { ViewOpened } from './bindings/ViewOpened';
import type { VolumeEvent } from './bindings/VolumeEvent';
import { cmd } from './command-spec';
import type { EventSink } from './transport';

/**
 * Where the folder of search results is, for the backend to check on the way to it (see `within.rs`): `dir` is its path
 * below the folder searched `root`, names joined with `/` (`''` is the root itself).
 */
type ResultFolder = { root: string; dir: string };

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
  archive_open: cmd<
    { pane: PaneId; archive: string; dir: string; showHidden: boolean; onEvent: EventSink<ListEvent> },
    null
  >(),
  archive_password: cmd<{ archive: string; password: string }, boolean>(),
  pane_folder_size: cmd<{ pane: PaneId; path: string; name: string; onEvent: EventSink<SizeEvent> }, null>(),
  find_start: cmd<{ query: FindQuery; onEvent: EventSink<FindEvent> }, FindId>(),
  find_cancel: cmd<{ id: FindId }, null>(),
  find_forget: cmd<{ id: FindId }, null>(),
  find_open: cmd<{ pane: PaneId; id: FindId; onEvent: EventSink<ListEvent> }, null>(),
  compare_start: cmd<
    { left: CompareRoot; right: CompareRoot; options: CompareOptions; onEvent: EventSink<CompareEvent> },
    CompareId
  >(),
  compare_cancel: cmd<{ id: CompareId }, null>(),
  compare_forget: cmd<{ id: CompareId }, null>(),
  volumes_list: cmd<{ onEvent: EventSink<VolumeEvent> }, null>(),
  ops_subscribe: cmd<{ onEvent: EventSink<OpsEvent> }, null>(),
  ops_start: cmd<{ kind: OpKind; from: string; names: string[]; to: string; verify: boolean; within?: ResultFolder }, JobId>(),
  ops_extract: cmd<{ archive: string; dir: string; names: string[]; to: string }, JobId>(),
  ops_task: cmd<{ from: string; names: string[]; task: JobTask }, JobId>(),
  ops_pack: cmd<
    {
      from: string;
      names: string[];
      archive: string;
      format: PackFormat;
      level: PackLevel;
      password: string | null;
      encryptNames: boolean;
    },
    JobId
  >(),
  ops_add_to_archive: cmd<{ archive: string; dir: string; from: string; names: string[] }, JobId>(),
  ops_control: cmd<{ job: JobId; action: ControlAction }, null>(),
  ops_answer: cmd<{ job: JobId; question: number; answer: Answer; all: boolean }, null>(),
  ops_rename: cmd<{ path: string; from: string; to: string }, null>(),
  rename_preview: cmd<{ path: string; names: string[]; rule: RenameRule }, RenamePreview>(),
  ops_sync_undo: cmd<{ job: JobId }, JobId>(),
  ops_rename_undo: cmd<{ job: JobId }, JobId>(),
  ops_sync: cmd<{ scan: CompareId; left: CompareRoot; right: CompareRoot; items: SyncItem[]; removals: SyncRemoval[]; permanent: boolean }, JobId>(),
  ops_trash: cmd<{ path: string; names: string[]; within?: ResultFolder }, TrashOutcome>(),
  ops_untrash: cmd<{ token: number }, number>(),
  ops_delete: cmd<{ path: string; names: string[]; within?: ResultFolder }, null>(),
  ops_mkdir: cmd<{ path: string; name: string }, string>(),
  ops_mkfile: cmd<{ path: string; name: string }, boolean>(),
  ops_properties: cmd<{ path: string; name: string }, ItemProperties>(),
  ops_link: cmd<{ kind: LinkKind; path: string; name: string; to: string; linkName: string }, null>(),
  view_open: cmd<{ path: string; name: string }, ViewOpened>(),
  archive_view_open: cmd<{ archive: string; dir: string; name: string }, ViewOpened>(),
  view_close: cmd<{ session: number }, null>(),
  diff_next_difference: cmd<{ left: number; right: number; from: number; forward: boolean }, ByteDifference>(),
  view_bytes: cmd<{ session: number; offset: number; length: number }, ArrayBuffer>(),
  view_image_decode: cmd<{ session: number }, ArrayBuffer>(),
  view_text: cmd<{ session: number; query: TextQuery }, TextWindow>(),
  view_search: cmd<{ session: number; query: SearchQuery; onProgress: EventSink<number> }, SearchHit | null>(),
  view_search_cancel: cmd<{ session: number }, null>(),
  view_markdown: cmd<{ session: number; encoding: string }, MdDocument>(),
  view_archive: cmd<{ session: number }, ArchiveListing>(),
  view_media: cmd<{ session: number }, string>(),
  edit_open: cmd<{ path: string; name: string; encoding: string | null }, EditOpened>(),
  archive_edit_open: cmd<{ archive: string; dir: string; name: string; encoding: string | null }, EditOpened>(),
  edit_save: cmd<{ path: string; name: string; text: string; options: EditSaveOptions }, EditSaved>(),
  edit_external: cmd<{ path: string; name: string }, ExternalEdit>(),
  settings_open: cmd<undefined, null>(),
  editor_set_dirty: cmd<{ dirty: boolean }, null>(),
  editor_close_ack: cmd<undefined, null>(),
  app_close_confirmed: cmd<undefined, null>(),
  state_load: cmd<undefined, SavedState | null>(),
  state_save: cmd<{ state: SavedState }, null>(),
};

export type CommandName = keyof typeof commands;
export type CommandArgs<K extends CommandName> = (typeof commands)[K]['args'];
export type CommandResult<K extends CommandName> = (typeof commands)[K]['result'];
