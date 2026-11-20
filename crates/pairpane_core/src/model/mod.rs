//! IPC data-transfer objects shared with the webview.
//!
//! Every type here derives `serde` (camelCase on the wire) and `ts_rs::TS`; `#[ts(export)]`
//! writes one TypeScript file per type into `ui/src/lib/ipc/bindings` when
//! `cargo test -p pairpane_core` runs. The generated files are never edited by hand.

mod app;
mod ipc_error;

pub use app::AppInfo;
pub use ipc_error::{IpcError, IpcErrorCode};
