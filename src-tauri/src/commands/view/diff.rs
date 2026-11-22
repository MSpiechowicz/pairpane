//! The byte diff of two sessions: where the files of two open sessions next differ (the hex diff's Previous and Next).
//!
//! Each call scans at most [`DIFF_BUDGET`] bytes of each file and then answers `searching` with where to go on, so a
//! pair of multi-gigabyte files never holds a thread for long and the page can show progress and stop asking. The scan
//! runs on a place of the search cap (so the windows of the diff keep reading on the read cap) and stops before its
//! next chunk when either session ends.

use std::sync::atomic::Ordering;

use pairpane_core::model::ByteDifference;
use pairpane_core::view::{DIFF_BUDGET, next_difference};
use tauri::State;

use super::read::run;
use super::{SEARCH_THREADS, Viewer};
use crate::error::CommandResult;

impl Viewer {
    /// The next (or, not `forward`, previous) difference between the files of `left` and `right`, from `from`.
    pub(super) async fn next_difference(
        &self,
        left: u32,
        right: u32,
        from: u64,
        forward: bool,
    ) -> CommandResult<ByteDifference> {
        let (left, right) = (self.handle(left)?, self.handle(right)?);

        run(&self.searches, SEARCH_THREADS, left, move |file, left_cancel| {
            let stopped = || left_cancel.load(Ordering::Relaxed) || right.cancel.load(Ordering::Relaxed);

            next_difference(file, &right.file, from, forward, DIFF_BUDGET, &stopped)
        })
        .await
    }
}

/// Where the files of the sessions `left` and `right` next differ, going `forward` from `from` (the first offset at or
/// after it) or backward (the last offset before it). A byte that only the longer file has counts as a difference.
/// The answer is `found`, `none` (nothing in that direction) or `searching` with the offset to ask again from when the
/// scan of this call ran out of budget. A session that is not there is `notFound`; one that is closed meanwhile ends
/// the scan as a cancelled read.
#[tauri::command]
pub async fn diff_next_difference(
    viewer: State<'_, Viewer>,
    left: u32,
    right: u32,
    from: u64,
    forward: bool,
) -> CommandResult<ByteDifference> {
    viewer.next_difference(left, right, from, forward).await
}
