//! What the comparison of the work tree with the index may read: a file that is not a regular file, or is over the cap, is
//! `modified` and is never opened, and a file that was changed within the second the index was written is read.

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use gix::index::entry::Stage;

use super::fixture::{Repo, Staged, pairs, within};
use super::testing::status;
use super::tests::{clean, marks_with, owned};
use super::*;
use crate::model::GitMark::{Conflicted, Modified, Staged as Added};

fn marks(folder: &Path, limits: Limits) -> Vec<(String, crate::model::GitMark)> {
    marks_with(folder, limits)
}

#[test]
fn a_file_over_the_cap_is_modified_without_being_read_and_under_it_is_compared_by_its_content() {
    let repo = clean(&[("big.txt", &[b'x'; 100]), ("small.txt", b"s")]);
    // The index has no stat of the files, so each is compared by its content when it can be.
    let capped = Limits {
        compare_bytes: 50,
        ..Limits::default()
    };

    assert_eq!(marks(repo.root(), capped), owned(&[("big.txt", Modified)]));
    assert_eq!(marks(repo.root(), Limits::default()), vec![]);
}

#[test]
fn a_file_changed_within_the_second_of_the_index_is_found_and_one_that_is_not_racy_is_not_read() {
    let repo = clean(&[("a.txt", b"aaaa")]);
    // The file changes to another content of the same size, with the time it had; the index has its stat as it now is, and the
    // content of before (this is what `git add` followed by an edit in the same clock tick leaves).
    repo.put("a.txt", b"bbbb");
    let time = SystemTime::now() - Duration::from_secs(3600);
    let set_time = |path: &Path, time: SystemTime| {
        fs::File::options()
            .write(true)
            .open(path)
            .expect("the file")
            .set_modified(time)
            .expect("its time");
    };
    set_time(&repo.path("a.txt"), time);
    repo.write_index_with_disk_stat(&[Staged::file("a.txt", b"aaaa")]);

    // The index is as old as the file: racy, so the file is read and found changed.
    set_time(&repo.index_path(), time);
    assert_eq!(marks(repo.root(), Limits::default()), owned(&[("a.txt", Modified)]));

    // The index is newer than the file by some time: not racy, so the stat is believed (as git does).
    set_time(&repo.index_path(), time + Duration::from_secs(60));
    assert_eq!(marks(repo.root(), Limits::default()), vec![]);
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::view::testing::fifo;

    /// The marks of the repository's folder, read on a thread that has to answer.
    fn marks_within(repo: &Repo) -> Vec<(String, crate::model::GitMark)> {
        let root = repo.root().to_path_buf();

        within(10, move || marks(&root, Limits::default()))
    }

    /// A named pipe in the place of the tracked file `name`, with the mode `mode`.
    fn pipe_for(repo: &Repo, name: &str, mode: u32) {
        repo.remove(name);
        fifo(repo.root(), name);
        fs::set_permissions(repo.path(name), fs::Permissions::from_mode(mode)).expect("a mode");
    }

    #[test]
    fn a_named_pipe_in_the_place_of_an_executable_tracked_file_is_modified_and_is_not_opened() {
        for mode in [0o755, 0o644] {
            let repo = Repo::new();
            repo.write_index(&[Staged::executable("run.sh", b"#!"), Staged::file("keep.txt", b"k")]);
            repo.put("keep.txt", b"k");
            repo.put("run.sh", b"#!");
            pipe_for(&repo, "run.sh", mode);

            let answer = marks_within(&repo);

            // The index is not in `HEAD` (it has none), so both are staged; the pipe is a change as well.
            assert_eq!(
                answer,
                owned(&[("keep.txt", Added), ("run.sh", Modified)]),
                "mode {mode:o}"
            );
        }
    }

    #[test]
    fn a_named_pipe_in_the_place_of_a_plain_tracked_file_is_modified() {
        let repo = clean(&[("a.txt", b"a"), ("b.txt", b"b")]);
        pipe_for(&repo, "a.txt", 0o755);

        let answer = marks_within(&repo);

        assert_eq!(answer, owned(&[("a.txt", Modified)]));
    }

    #[test]
    fn a_link_to_a_device_in_the_place_of_a_tracked_file_is_modified_and_not_followed() {
        let repo = clean(&[("a.txt", b"a")]);
        repo.remove("a.txt");
        std::os::unix::fs::symlink("/dev/zero", repo.path("a.txt")).expect("a link");

        let answer = marks_within(&repo);

        assert_eq!(answer, owned(&[("a.txt", Modified)]));
    }

    #[test]
    fn a_tracked_link_whose_target_is_unchanged_is_still_compared_as_a_link() {
        let repo = Repo::new();
        repo.write_index(&[Staged {
            mode: gix::index::entry::Mode::SYMLINK,
            ..Staged::file("l", b"target")
        }]);
        std::os::unix::fs::symlink("target", repo.path("l")).expect("a link");

        // It is in the index and not in `HEAD`: staged, and not modified.
        assert_eq!(marks(repo.root(), Limits::default()), owned(&[("l", Added)]));

        fs::remove_file(repo.path("l")).expect("removed");
        std::os::unix::fs::symlink("elsewhere", repo.path("l")).expect("a link");
        assert_eq!(marks(repo.root(), Limits::default()), owned(&[("l", Modified)]));
    }

    #[test]
    fn a_named_pipe_for_gitattributes_above_a_changed_file_is_not_opened() {
        let repo = clean(&[("sub/a.txt", b"a"), ("sub/b.txt", b"b"), ("top.txt", b"t")]);
        repo.put("sub/a.txt", b"a, changed");
        fifo(&repo.path("sub"), ".gitattributes");

        let answer = marks_within(&repo);

        // `a.txt` could not be compared (its `.gitattributes` would be read), so it is modified, and the pipe itself is an untracked file.
        assert_eq!(answer, owned(&[("sub", Modified)]));
    }

    #[test]
    fn a_named_pipe_for_gitignore_keeps_the_walk_out_of_its_folder_and_the_folder_has_no_status() {
        let repo = clean(&[("sub/a.txt", b"a"), ("top.txt", b"t")]);
        repo.put("sub/new.txt", b"untracked");
        fifo(&repo.path("sub"), ".gitignore");
        let (root, sub) = (repo.root().to_path_buf(), repo.path("sub"));

        let (from_root, from_sub) = within(10, move || {
            (
                status(&root, Limits::default(), &AtomicBool::new(false)).expect("a status"),
                status(&sub, Limits::default(), &AtomicBool::new(false)).expect("a status"),
            )
        });

        // From the root the folder is not entered, and is `modified` as the safe thing to say of what was not looked at; from the
        // folder itself it has no status (a pipe is no rules file, which is read for every entry of the folder).
        assert!(from_root.repo);
        assert_eq!(pairs(&from_root.marks), vec![("sub", Modified)]);
        assert!(!from_sub.repo);
    }

    #[test]
    fn an_entry_in_conflict_is_not_staged_and_a_gitlink_that_changed_is() {
        let repo = clean(&[("a.txt", b"a")]);
        repo.write_index(&[
            Staged::file("a.txt", b"a"),
            Staged::gitlink("vendor/lib"),
            Staged::conflict("z.txt", b"z", Stage::Ours),
        ]);
        repo.put("z.txt", b"z");
        fs::create_dir_all(repo.path("vendor/lib")).expect("the submodule folder");

        assert_eq!(
            marks(repo.root(), Limits::default()),
            owned(&[("vendor", Added), ("z.txt", Conflicted)])
        );
        assert_eq!(marks(&repo.path("vendor"), Limits::default()), owned(&[("lib", Added)]));
    }
}
