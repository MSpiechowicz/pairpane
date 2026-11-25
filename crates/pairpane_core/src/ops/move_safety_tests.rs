//! A move across disks keeps its source whenever the copy might not be whole: a flush that fails, and a source that
//! changed while it was copied (data loss audit, #68).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::Arc;

use pretty_assertions::assert_eq;

use crate::model::Outcome;

use super::testing::{Rig, has_no_partial_files, pattern, read, temp, tree, write};

#[test]
fn a_copy_that_cannot_be_flushed_to_the_disk_keeps_the_source() {
    let root = temp();
    let src = root.path().join("src");
    write(&src.join("a.bin"), &pattern(256 * 1024));
    let mut rig = Rig::hooked(|hooks| {
        hooks.cross_device = true;
        hooks.fail_flush = true;
    });

    let ended = rig.run_move(root.path(), &["a.bin"]);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 1, "{:?}", ended.errors);
    assert_eq!(read(&src.join("a.bin")), pattern(256 * 1024));
    assert_eq!(tree(&root.path().join("dst")), Vec::<String>::new());
    assert!(has_no_partial_files(root.path()));
}

#[test]
fn a_source_that_grew_while_it_was_moved_is_kept() {
    let root = temp();
    let src = root.path().join("src");
    write(&src.join("log.txt"), b"first line\n");
    let mut rig = Rig::hooked(|hooks| {
        hooks.cross_device = true;
        hooks.before_removal = Some(Arc::new(|source| {
            let mut file = OpenOptions::new().append(true).open(source).expect("source");
            file.write_all(b"written during the move\n").expect("append");
        }));
    });

    let ended = rig.run_move(root.path(), &["log.txt"]);

    assert_eq!(ended.errors.len(), 1, "{:?}", ended.errors);
    assert_eq!(ended.errors[0].path, "log.txt");
    assert_eq!(
        ended.errors[0].error.message,
        "Kept the original: it changed while it was being moved"
    );
    assert_eq!(read(&src.join("log.txt")), b"first line\nwritten during the move\n");
    assert_eq!(read(&root.path().join("dst/log.txt")), b"first line\n");
}

#[test]
fn a_source_replaced_under_its_name_while_it_was_moved_is_kept() {
    let root = temp();
    let src = root.path().join("src");
    write(&src.join("a.txt"), b"old");
    let mut rig = Rig::hooked(|hooks| {
        hooks.cross_device = true;
        hooks.before_removal = Some(Arc::new(|source| {
            let new = source.with_file_name("new.txt");
            fs::write(&new, b"new").expect("new file");
            fs::rename(&new, source).expect("replace");
        }));
    });

    let ended = rig.run_move(root.path(), &["a.txt"]);

    assert_eq!(ended.errors.len(), 1, "{:?}", ended.errors);
    assert_eq!(read(&src.join("a.txt")), b"new");
    assert_eq!(read(&root.path().join("dst/a.txt")), b"old");
}

#[cfg(unix)]
#[test]
fn a_pipe_swapped_in_for_the_target_folder_does_not_block_the_move() {
    use std::process::Command;

    let root = temp();
    let src = root.path().join("src");
    write(&src.join("a.txt"), b"a");
    let mut rig = Rig::hooked(|hooks| {
        hooks.cross_device = true;
        hooks.before_removal = Some(Arc::new(|source| {
            let dst = source.parent().and_then(|src| src.parent()).expect("root").join("dst");
            fs::rename(&dst, dst.with_file_name("dst-moved")).expect("move the folder away");
            let made = Command::new("mkfifo").arg(&dst).status().expect("mkfifo");
            assert!(made.success());
        }));
    });

    // Opening the pipe as a folder would wait for a writer for good; the rig gives up on a job that does not end.
    let ended = rig.run_move(root.path(), &["a.txt"]);

    assert_eq!(ended.errors.len(), 1, "{:?}", ended.errors);
    assert_eq!(read(&src.join("a.txt")), b"a");
}
