//! IPC data-transfer objects shared with the webview.
//!
//! Every type here derives `serde` (camelCase on the wire) and `ts_rs::TS`; `#[ts(export)]`
//! writes one TypeScript file per type into `ui/src/lib/ipc/bindings` when
//! `cargo test -p pairpane_core` runs. The generated files are never edited by hand.

mod app;
mod archive;
mod edit;
mod find;
mod ipc_error;
mod items;
mod listing;
mod markdown;
mod ops;
mod pack;
mod rename;
mod state;
mod tasks;
mod update;
mod view;
mod volume;

pub use app::AppInfo;
pub use archive::ArchiveSpot;
pub use edit::{EditDocument, EditOpened, EditSaveOptions, EditSaved, ExternalEdit, LineEnding};
pub use find::{FindCriteria, FindEvent, FindHit, FindId, FindQuery, FindText, SavedSearch, Tri};
pub use ipc_error::{IpcError, IpcErrorCode};
pub use items::{LinkKind, TrashOutcome};
pub use listing::{Crumb, Entry, EntryKind, ListEvent, PaneId, SizeEvent};
pub use markdown::{MdBlock, MdDocument, MdInline, MdListItem};
pub use ops::{
    Answer, ConflictInfo, ControlAction, FileFacts, ItemError, JobId, JobPhase, JobView, OpKind, OpsEvent, Outcome,
};
pub use pack::{PackFormat, PackLevel};
pub use rename::{
    CaseChange, CasePart, RenameCounter, RenamePreview, RenameProblem, RenameRow, RenameRule, RuleError, RuleField,
    SavedRename,
};
pub use state::{SavedPane, SavedState, SavedTab, SortColumn};
pub use tasks::{AttributeChange, HashAlgorithm, ItemProperties, JobTask, ModeChange, WindowsAttributes};
pub use update::{MAX_UPDATE_NOTES_BYTES, MenuCommand, UpdateInfo, UpdatePhase, UpdateProgress};
pub use view::{
    ArchiveEntry, ArchiveFormat, ArchiveListing, SearchHit, SearchNeedle, SearchQuery, TextLine, TextQuery, TextWindow,
    ViewKind, ViewOpened,
};
pub use volume::{Volume, VolumeEvent, VolumeKind};
