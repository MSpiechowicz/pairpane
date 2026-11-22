//! Compare and synchronize folders: `compare_start` scans two folders on a blocking thread and streams its
//! [`CompareEvent`]s over the channel it is given; `compare_cancel` stops it, `compare_forget` drops it, and `ops_sync`
//! queues one job that copies the items the page chose from the scan.
//!
//! Not built yet: every command answers `unsupported`.

use pairpane_core::model::{CompareEvent, CompareId, CompareOptions, CompareRoot, IpcErrorCode, JobId, SyncItem};
use tauri::State;
use tauri::ipc::Channel;

use crate::error::{CommandError, CommandResult};

/// What every command answers until the scan and the job exist.
const NOT_READY: &str = "Compare is not ready";

/// The comparisons of the app. Holds nothing yet; the scan's bookkeeping lands here, built with `default()`.
#[derive(Default)]
pub struct Comparer {
    _scans: (),
}

impl Comparer {
    /// Stops every comparison, as the app exits.
    pub fn shut_down(&self) {}
}

fn not_ready() -> CommandError {
    CommandError::new(IpcErrorCode::Unsupported, NOT_READY)
}

/// Starts the comparison of `left` and `right` and answers with its id at once; the events follow on `on_event`.
#[tauri::command]
pub async fn compare_start(
    comparer: State<'_, Comparer>,
    left: CompareRoot,
    right: CompareRoot,
    options: CompareOptions,
    on_event: Channel<CompareEvent>,
) -> CommandResult<CompareId> {
    let _ = (&comparer, &left, &right, &options, &on_event);

    Err(not_ready())
}

/// Stops the comparison `id` (its `done` says `cancelled`); its rows stay until it is forgotten.
#[tauri::command]
pub async fn compare_cancel(comparer: State<'_, Comparer>, id: CompareId) -> CommandResult<()> {
    let _ = (&comparer, id);

    Err(not_ready())
}

/// Stops the comparison `id` and drops what it kept.
#[tauri::command]
pub async fn compare_forget(comparer: State<'_, Comparer>, id: CompareId) -> CommandResult<()> {
    let _ = (&comparer, id);

    Err(not_ready())
}

/// Queues the one job that copies `items` between `left` and `right`, each the way its direction says.
#[tauri::command]
pub async fn ops_sync(left: CompareRoot, right: CompareRoot, items: Vec<SyncItem>) -> CommandResult<JobId> {
    let _ = (&left, &right, &items);

    Err(not_ready())
}

#[cfg(test)]
mod tests {
    use pairpane_core::model::IpcErrorCode;

    use super::*;

    #[test]
    fn the_stub_refuses_as_unsupported() {
        let error = not_ready();

        assert_eq!(error.code(), IpcErrorCode::Unsupported);
    }
}
