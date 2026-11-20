//! Whole move jobs on real folders, and cancelling a job in the middle of a file.

use std::fs;
use std::path::Path;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::model::{Answer, ControlAction, JobPhase, OpsEvent, Outcome};

use super::hooks::Hooks;
use super::testing::{
    Ended, Rig, copy, has_no_partial_files, long_ago, modified, pattern, phases, read, relocate, set_times, temp, tree,
    write,
};

#[test]
fn a_move_within_one_volume_renames_without_copying() {
    let root = temp();
    let src = root.path().join("src");
    write(&src.join("a.txt"), b"a");
    write(&src.join("folder/b.txt"), b"b");
    let before = inodes(&[&src.join("a.txt"), &src.join("folder")]);
    let mut rig = Rig::new();

    let ended = rig.run_move(root.path(), &["a.txt", "folder"]);

    let dst = root.path().join("dst");
    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(tree(&src), Vec::<String>::new());
    assert_eq!(tree(&dst), ["a.txt", "folder/", "folder/b.txt"]);
    assert_eq!(inodes(&[&dst.join("a.txt"), &dst.join("folder")]), before);
    assert_eq!(phases(&rig), [JobPhase::Preparing, JobPhase::Renaming]);
}

/// The inode of each path on Unix; nothing elsewhere.
fn inodes(paths: &[&Path]) -> Vec<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        paths
            .iter()
            .map(|path| fs::metadata(path).expect("metadata").ino())
            .collect()
    }
    #[cfg(not(unix))]
    {
        let _ = paths;
        Vec::new()
    }
}

#[test]
fn a_move_across_disks_copies_then_deletes_each_source() {
    let root = temp();
    let src = root.path().join("src");
    write(&src.join("a.txt"), &pattern(1024 * 1024 + 5));
    write(&src.join("folder/b.txt"), b"b");
    write(&src.join("folder/deep/c.txt"), b"c");
    fs::create_dir_all(src.join("folder/empty")).expect("folder");
    set_times(&src.join("a.txt"), long_ago());
    let original = read(&src.join("a.txt"));
    let mut rig = Rig::hooked(|hooks| hooks.cross_device = true);

    let ended = rig.run_move(root.path(), &["a.txt", "folder"]);

    let dst = root.path().join("dst");
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(read(&dst.join("a.txt")), original);
    assert_eq!(modified(&dst.join("a.txt")), long_ago());
    assert_eq!(
        tree(&dst),
        [
            "a.txt",
            "folder/",
            "folder/b.txt",
            "folder/deep/",
            "folder/deep/c.txt",
            "folder/empty/"
        ]
    );
    assert_eq!(
        tree(&src),
        Vec::<String>::new(),
        "every source is gone once its copy is committed"
    );
    assert!(phases(&rig).contains(&JobPhase::Copying));
}

#[cfg(unix)]
#[test]
fn a_move_between_two_real_filesystems_falls_back_to_copy_and_delete() {
    use std::os::unix::fs::MetadataExt;

    let root = temp();
    let Ok(shared) = tempfile::tempdir_in("/dev/shm") else {
        return;
    };
    if fs::metadata(root.path()).expect("metadata").dev() == fs::metadata(shared.path()).expect("metadata").dev() {
        return;
    }
    let src = root.path().join("src");
    write(&src.join("a.txt"), b"across");
    write(&src.join("folder/b.txt"), b"b");
    let mut rig = Rig::new();

    let ended = rig.run(relocate(&src, &["a.txt", "folder"], shared.path()));

    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(read(&shared.path().join("a.txt")), b"across");
    assert_eq!(read(&shared.path().join("folder/b.txt")), b"b");
    assert_eq!(tree(&src), Vec::<String>::new());
}

#[test]
fn cancelling_from_the_progress_stops_within_a_chunk_and_leaves_nothing_behind() {
    let root = temp();
    let src = root.path().join("src");
    write(&src.join("tree/big.bin"), &vec![7; 64 * 1024 * 1024]);
    // The listener cancels at the first progress that shows bytes written, so the cancel always lands mid-file.
    let mut rig = Rig::with(
        Hooks {
            every_chunk: true,
            ..Hooks::default()
        },
        |engine, event| {
            if let OpsEvent::Progress { id, done_bytes, .. } = event
                && *done_bytes > 1
            {
                let _ = engine.control(*id, ControlAction::Cancel);
            }
        },
    );

    let ended = rig.run(copy(&src, &["tree"], &root.path().join("made/by/the/job")));

    assert_eq!(ended.outcome, Outcome::Cancelled);
    assert!(
        !root.path().join("made").exists(),
        "the folders the job made are removed: {:?}",
        tree(root.path())
    );
    assert_eq!(read(&src.join("tree/big.bin")).len(), 64 * 1024 * 1024);
    assert!(has_no_partial_files(root.path()));
}

/// A rig for a move between two disks whose listener cancels the job at the first progress that shows bytes written,
/// so the cancel always lands in the middle of a file.
fn cancelling_cross_device_rig() -> Rig {
    Rig::with(
        Hooks {
            every_chunk: true,
            cross_device: true,
            ..Hooks::default()
        },
        |engine, event| {
            if let OpsEvent::Progress { id, done_bytes, .. } = event
                && *done_bytes > 5
            {
                let _ = engine.control(*id, ControlAction::Cancel);
            }
        },
    )
}

#[test]
fn a_cancelled_move_loses_nothing_and_keeps_what_was_finished() {
    let root = temp();
    let src = root.path().join("src");
    write(&src.join("a-small.txt"), b"small");
    write(&src.join("b-big.bin"), &vec![7; 32 * 1024 * 1024]);
    let mut rig = cancelling_cross_device_rig();

    let ended = rig.run(relocate(&src, &["a-small.txt", "b-big.bin"], &root.path().join("dst")));

    assert_eq!(ended.outcome, Outcome::Cancelled);
    // Whichever file the cancel interrupted is whole at its source; one that was finished is whole at the target
    // and gone from the source; nothing is lost, and nothing half-written is left.
    for (name, len) in [("a-small.txt", 5), ("b-big.bin", 32 * 1024 * 1024)] {
        let source_len = fs::metadata(src.join(name)).map(|metadata| metadata.len());
        let target_len = fs::metadata(root.path().join("dst").join(name)).map(|metadata| metadata.len());

        assert!(
            source_len.ok() == Some(len) || target_len.ok() == Some(len),
            "{name} was lost: {:?}",
            tree(root.path())
        );
    }
    assert!(has_no_partial_files(root.path()));
}

#[test]
fn cancelling_a_move_across_disks_keeps_the_empty_folders_it_already_moved() {
    let root = temp();
    let src = root.path().join("src");
    fs::create_dir_all(src.join("a-empty")).expect("folder");
    write(&src.join("b-big.bin"), &vec![7; 32 * 1024 * 1024]);
    let mut rig = cancelling_cross_device_rig();

    let ended = rig.run(relocate(&src, &["a-empty", "b-big.bin"], &root.path().join("dst")));

    assert_eq!(ended.outcome, Outcome::Cancelled);
    // The folder was moved before the cancel: its source is gone, so the target is the only one left.
    assert!(!src.join("a-empty").exists());
    assert!(root.path().join("dst/a-empty").is_dir(), "{:?}", tree(root.path()));
    assert_eq!(read(&src.join("b-big.bin")).len(), 32 * 1024 * 1024);
    assert!(has_no_partial_files(root.path()));
}

/// Moves `a.txt` into its own folder with a rig set by `set`, and answers "overwrite" to the question about the file
/// that is already there: it is the file itself. Returns how the job ended, and the folder.
fn move_onto_itself(set: impl FnOnce(&mut Hooks)) -> (Ended, TempDir) {
    let root = temp();
    let src = root.path().join("src");
    write(&src.join("a.txt"), b"only copy");
    let mut rig = Rig::hooked(set);

    let job = rig.start(relocate(&src, &["a.txt"], &src));
    rig.answer_next(job, Answer::Overwrite, false);
    let ended = rig.wait_ended(job);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(read(&src.join("a.txt")), b"only copy");
    assert_eq!(tree(&src), ["a.txt"]);

    (ended, root)
}

#[test]
fn a_move_never_deletes_a_source_that_is_the_committed_target() {
    // A move into its own folder, as a name nothing recognises (the check of the conflict sees nothing): the answer
    // "overwrite" replaces the file with a copy of itself, and the source must not then be deleted.
    // (A rename onto itself would end the move without a copy; pretend it fails like one between two disks.)
    let (ended, _root) = move_onto_itself(|hooks| {
        hooks.blind_identity = true;
        hooks.cross_device = true;
    });

    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
}

#[test]
fn a_move_whose_identities_cannot_be_read_replaces_nothing() {
    // The names alone would show that the target is the source's own folder; a missing identity still counts as
    // "the same file" for the question about overwriting, so the answer "overwrite" becomes a skip.
    let (ended, _root) = move_onto_itself(|hooks| hooks.unreadable_identity = true);

    assert_eq!(ended.skipped, 1);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
}

#[test]
fn a_move_keeps_the_original_when_it_cannot_confirm_the_copy_is_a_different_file() {
    // The question about overwriting is fooled (the file replaces itself), and the identities cannot be read to
    // tell the copy from its source afterwards: the source stays, and the item says why.
    let (ended, _root) = move_onto_itself(|hooks| {
        hooks.blind_identity = true;
        hooks.unreadable_identity = true;
        hooks.cross_device = true;
    });

    assert_eq!(ended.errors.len(), 1);
    assert_eq!(ended.errors[0].path, "a.txt");
    assert_eq!(
        ended.errors[0].error.message,
        "Kept the original: could not confirm the copy is a different file"
    );
}

#[test]
fn overwriting_a_file_with_itself_is_skipped_for_a_hard_link_too() {
    let root = temp();
    write(&root.path().join("src/a.txt"), b"one file");
    fs::create_dir_all(root.path().join("dst")).expect("folder");
    fs::hard_link(root.path().join("src/a.txt"), root.path().join("dst/a.txt")).expect("hard link");
    let mut rig = Rig::new();

    let job = rig.start(relocate(&root.path().join("src"), &["a.txt"], &root.path().join("dst")));
    rig.answer_next(job, Answer::Overwrite, false);
    let ended = rig.wait_ended(job);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.skipped, 1);
    assert_eq!(read(&root.path().join("src/a.txt")), b"one file");
    assert_eq!(read(&root.path().join("dst/a.txt")), b"one file");
}

/// Moves `src/<name>` into `to`, a spelling of the folder `src` itself, and answers "overwrite" to the question
/// about the file that is already there: it is the file itself. Nothing may be lost.
fn move_a_file_onto_itself_through(root: &Path, name: &str, to: &Path) {
    let mut rig = Rig::new();

    let job = rig.start(relocate(&root.join("src"), &[name], to));
    rig.answer_next(job, Answer::Overwrite, false);
    let ended = rig.wait_ended(job);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.skipped, 1, "the file is left as it is");
    assert_eq!(read(&root.join("src").join(name)), b"only copy");
    assert_eq!(tree(&root.join("src")), [name]);
}

#[cfg(unix)]
#[test]
fn moving_a_file_onto_itself_through_a_link_to_its_folder_loses_nothing() {
    let root = temp();
    write(&root.path().join("src/a.txt"), b"only copy");
    let alias = root.path().join("alias");
    std::os::unix::fs::symlink(root.path().join("src"), &alias).expect("link");

    move_a_file_onto_itself_through(root.path(), "a.txt", &alias);
}

#[cfg(windows)]
#[test]
fn moving_a_file_onto_itself_through_another_spelling_of_its_folder_loses_nothing() {
    use super::testing::other_spelling;

    let root = temp();
    write(&root.path().join("src/a.txt"), b"only copy");
    let Some(other) = other_spelling(root.path()) else {
        return;
    };
    // The same folder with its letters in another case, which a comparison by name would also have to get right.
    let other_case = std::path::PathBuf::from(root.path().to_string_lossy().to_uppercase()).join("SRC");

    for to in [other.join("src"), other_case] {
        move_a_file_onto_itself_through(root.path(), "a.txt", &to);
    }
}
