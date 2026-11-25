//! The old file a synchronization replaces stays kept for the undo even when the new one does not match the plan's
//! source (data loss audit, #68).

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use pretty_assertions::assert_eq;

use crate::model::{OpsEvent, SyncDirection};

use super::hooks::Hooks;
use super::sync_tests::{Pair, file, is_done, plan};
use super::sync_undo_tests::undo_of;
use super::testing::{Rig, pattern, read, task};
use super::{SyncLog, SyncPlan, SyncRoot, Task};

#[test]
fn a_file_replaced_by_a_source_that_grew_meanwhile_can_still_be_undone() {
    let pair = Pair::new();
    let big = pattern(3 * 1024 * 1024);
    file(&pair.left().join("over.bin"), &big, 300);
    file(&pair.right().join("over.bin"), b"older", 200);
    let plans: Vec<SyncPlan> = vec![plan(&pair.left(), &pair.right(), "over.bin", SyncDirection::ToRight)];
    let log = SyncLog::default();

    // The source grows while its first chunk is copied, so the copy holds more than the plan saw.
    let source = pair.left().join("over.bin");
    let grown = AtomicBool::new(false);
    let mut rig = Rig::with(
        Hooks {
            buffered_only: true,
            every_chunk: true,
            ..Hooks::default()
        },
        move |_, event| {
            if matches!(event, OpsEvent::Progress { done_bytes, .. } if *done_bytes > 0)
                && !grown.swap(true, Ordering::Relaxed)
            {
                let mut more = OpenOptions::new().append(true).open(&source).expect("source");
                more.write_all(b"more").expect("append");
            }
        },
    );

    let ended = rig.run(task(
        &pair.left(),
        &["over.bin"],
        &pair.right(),
        Task::Sync {
            left: SyncRoot::Folder,
            right: SyncRoot::Folder,
            plans,
            removals: Vec::new(),
            permanent: false,
            log: log.clone(),
        },
    ));

    is_done(&ended, 0);
    assert_eq!(
        read(&pair.right().join("over.bin")).len(),
        big.len() + 4,
        "the copy took the new bytes"
    );
    assert_eq!(log.undoable(), 1, "the replaced file is kept for the undo");

    let undone = Rig::new().run(undo_of(&pair, &log));

    is_done(&undone, 0);
    assert_eq!(read(&pair.right().join("over.bin")), b"older");
}
