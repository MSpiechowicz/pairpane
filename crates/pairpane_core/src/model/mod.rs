//! IPC data-transfer objects shared with the webview.
//!
//! Every type here derives `serde` (camelCase on the wire) and `ts_rs::TS`; `#[ts(export)]`
//! writes one TypeScript file per type into `ui/src/lib/ipc/bindings` when
//! `cargo test -p pairpane_core` runs. The generated files are never edited by hand.

mod app;
mod ipc_error;
mod items;
mod listing;
mod ops;
mod state;
mod tasks;
mod update;
mod volume;

pub use app::AppInfo;
pub use ipc_error::{IpcError, IpcErrorCode};
pub use items::{LinkKind, TrashOutcome};
pub use listing::{Crumb, Entry, EntryKind, ListEvent, PaneId, SizeEvent};
pub use ops::{
    Answer, ConflictInfo, ControlAction, FileFacts, ItemError, JobId, JobPhase, JobView, OpKind, OpsEvent, Outcome,
};
pub use state::{SavedPane, SavedState, SavedTab, SortColumn};
pub use tasks::{AttributeChange, HashAlgorithm, ItemProperties, JobTask, ModeChange, WindowsAttributes};
pub use update::{MAX_UPDATE_NOTES_BYTES, MenuCommand, UpdateInfo, UpdatePhase, UpdateProgress};
pub use volume::{Volume, VolumeEvent, VolumeKind};
