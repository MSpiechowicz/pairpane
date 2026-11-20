//! How one file is written: every data path gives the same bytes, a temporary file is never someone else's,
//! and a source that was swapped for a link or a pipe is never opened.

use std::fs;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

#[cfg(unix)]
use crate::model::Answer;
use crate::model::Outcome;

use super::copy::run_token;
#[cfg(unix)]
use super::open;
use super::testing::{Rig, has_no_partial_files, pattern, put, read, temp, tree, write};

#[test]
fn every_data_path_copies_the_same_bytes() {
    // 20 MiB and a bit: more than two kernel chunks, and not a multiple of the buffer.
    let sizes = [0, 1, 1024 * 1024, 20 * 1024 * 1024 + 3];

    for buffered_only in [false, true] {
        let root = temp();
        let names: Vec<String> = sizes.iter().map(|size| format!("f{size}")).collect();
        for (name, size) in names.iter().zip(sizes) {
            write(&root.path().join("src").join(name), &pattern(size));
        }
        let mut rig = Rig::hooked(|hooks| hooks.buffered_only = buffered_only);
        let names: Vec<&str> = names.iter().map(String::as_str).collect();

        let ended = rig.run_copy(root.path(), &names);

        assert_eq!(ended.outcome, Outcome::Done, "buffered only: {buffered_only}");
        assert!(ended.errors.is_empty(), "{:?}", ended.errors);
        for name in names {
            assert!(
                read(&root.path().join("dst").join(name)) == read(&root.path().join("src").join(name)),
                "{name} differs (buffered only: {buffered_only})"
            );
        }
        assert!(has_no_partial_files(root.path()));
    }
}

/// The first temporary file of the first job of a fresh engine, in `dst`.
fn first_temp(root: &TempDir) -> std::path::PathBuf {
    root.path().join(format!("dst/.pairpane-{}-1-1.part", run_token()))
}

/// `src/a.txt`, an empty `dst`, and a `victim` file that no copy may touch.
fn with_victim() -> TempDir {
    let root = temp();
    put(root.path(), "src/a.txt", b"new");
    put(root.path(), "victim", b"precious");
    fs::create_dir_all(root.path().join("dst")).expect("folder");

    root
}

/// Copies `a.txt` of `root` into its `dst`, which must go without a single error, and leaves the victim alone.
fn copy_past_what_is_planted(root: &TempDir) {
    let mut rig = Rig::new();

    let ended = rig.run_copy(root.path(), &["a.txt"]);

    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(read(&root.path().join("dst/a.txt")), b"new");
    assert_eq!(read(&root.path().join("victim")), b"precious");
}

#[test]
fn a_temporary_name_that_is_taken_is_never_opened_or_removed_and_the_next_one_is_used() {
    let root = with_victim();
    let planted = first_temp(&root);
    write(&planted, b"planted");

    copy_past_what_is_planted(&root);

    assert_eq!(read(&planted), b"planted");
}

#[test]
fn leftovers_of_an_earlier_run_do_not_fail_the_first_job() {
    let root = with_victim();
    // What a run that crashed left behind: job numbers start at 1 again with every start of the app.
    let stale = root.path().join("dst/.pairpane-1-1.part");
    write(&stale, b"stale");

    copy_past_what_is_planted(&root);

    assert_eq!(read(&stale), b"stale", "a file that is not ours is left alone");
}

#[cfg(unix)]
#[test]
fn a_link_planted_at_the_temporary_name_is_not_written_through() {
    let root = with_victim();
    let planted = first_temp(&root);
    std::os::unix::fs::symlink(root.path().join("victim"), &planted).expect("link");

    copy_past_what_is_planted(&root);

    assert!(fs::symlink_metadata(&planted).expect("still there").is_symlink());
}

#[cfg(unix)]
#[test]
fn a_target_that_is_a_link_is_replaced_never_written_through() {
    let root = with_victim();
    std::os::unix::fs::symlink(root.path().join("victim"), root.path().join("dst/a.txt")).expect("link");
    let mut rig = Rig::new();

    // Overwrite: the answer replaces the link itself.
    let job = rig.copy_in(root.path(), &["a.txt"]);
    rig.answer_next(job, Answer::Overwrite, false);
    rig.wait_ended(job);

    assert_eq!(read(&root.path().join("victim")), b"precious");
    assert_eq!(read(&root.path().join("dst/a.txt")), b"new");
    assert!(
        !fs::symlink_metadata(root.path().join("dst/a.txt"))
            .expect("metadata")
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn a_source_swapped_for_a_link_or_another_file_or_a_pipe_is_not_opened() {
    use rustix::fs::{CWD, FileType, Mode, mknodat};
    use std::os::unix::fs::symlink;

    let root = temp();
    let path = root.path().join("source");
    write(&path, b"real");
    let seen = fs::symlink_metadata(&path).expect("metadata");
    assert!(open::source(&path, &seen).is_ok());

    // A link in its place.
    write(&root.path().join("elsewhere"), b"other");
    fs::remove_file(&path).expect("remove");
    symlink(root.path().join("elsewhere"), &path).expect("link");
    assert!(open::source(&path, &seen).is_err());

    // Another file in its place: same name, different inode.
    fs::remove_file(&path).expect("remove");
    write(&path, b"swapped");
    assert!(open::source(&path, &seen).is_err());

    // A pipe in its place: refused at once, not waited on.
    fs::remove_file(&path).expect("remove");
    mknodat(CWD, &path, FileType::Fifo, Mode::RUSR | Mode::WUSR, 0).expect("fifo");
    assert!(open::source(&path, &seen).is_err());
}

#[test]
fn a_folder_in_the_way_of_a_file_fails_that_item_only() {
    let root = temp();
    write(&root.path().join("src/a.txt"), b"a");
    write(&root.path().join("src/b.txt"), b"b");
    fs::create_dir_all(root.path().join("dst/a.txt")).expect("folder");
    let mut rig = Rig::new();

    let ended = rig.run_copy(root.path(), &["a.txt", "b.txt"]);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 1);
    assert_eq!(ended.errors[0].path, "a.txt");
    assert_eq!(read(&root.path().join("dst/b.txt")), b"b");
    assert_eq!(tree(&root.path().join("dst")), ["a.txt/", "b.txt"]);
}

#[test]
fn an_already_exists_error_after_the_temporary_file_was_made_is_not_retried_as_a_taken_name() {
    let root = temp();
    put(root.path(), "src/a.bin", &pattern(1024));
    let mut rig = Rig::hooked(|hooks| hooks.taken_after_data = true);

    let ended = rig.run_copy(root.path(), &["a.bin"]);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 1);
    // Retried as a taken name, it would have failed after a hundred new names with "no free name was found".
    assert_eq!(ended.errors[0].error.message, "something with this name exists already");
    assert!(has_no_partial_files(root.path()));
    assert_eq!(tree(&root.path().join("dst")), Vec::<String>::new());
}
