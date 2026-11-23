//! What the directory walk does with the folders it enters: a nested repository (a folder of the work tree that has a `.git` of
//! its own, which is not a submodule) does not make it look at that `.git` (with `recurse_repositories` off the walk follows the
//! `gitdir:` pointer of the `.git` of each untracked folder and reads its `HEAD`, unchecked): the folder is walked as any other,
//! its `.git` is pruned, and it is `untracked` through its files. The children of a folder that is untracked as a whole are told
//! apart by the ignore rules, and the rules files of the folders it enters are bounded in size and in all.

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use super::fixture::within;
use super::tests::{clean, owned};
use super::*;
use crate::model::GitMark::{Ignored, Untracked};

fn answer(folder: &Path, limits: Limits) -> Result<GitStatus, GitError> {
    status(folder, limits, &AtomicBool::new(false))
}

/// The status of `folder`, which has to come on a thread of its own in time: the time limit of the status does not end a read
/// that waits for ever, so a test that hangs is the failure.
fn in_time(folder: &Path) -> Result<GitStatus, GitError> {
    let folder = folder.to_path_buf();

    within(10, move || answer(&folder, Limits::default()))
}

/// A folder `sub` of the repository's work tree that has a `.git` folder with an `objects` and `refs`, and a file.
fn nested_git_dir(repo: &super::fixture::Repo) -> std::path::PathBuf {
    let nested = repo.path("sub/.git");
    fs::create_dir_all(nested.join("objects")).expect("objects");
    fs::create_dir_all(nested.join("refs")).expect("refs");
    fs::write(repo.path("sub/x.txt"), b"x").expect("a file");

    nested
}

fn assert_sub_is_untracked(answer: Result<GitStatus, GitError>, what: &str) {
    let answer = answer.unwrap_or_else(|error| panic!("{what}: {error:?}"));

    assert!(answer.repo, "{what}");
    assert_eq!(super::fixture::pairs(&answer.marks), vec![("sub", Untracked)], "{what}");
}

#[test]
fn a_folder_that_is_untracked_as_a_whole_keeps_the_ignored_mark_of_its_children() {
    let repo = clean(&[(".gitignore", b"*.log\nbuild/\n"), ("a.txt", b"a")]);
    repo.put("fresh/new.txt", b"n");
    repo.put("fresh/debug.log", b"l");
    repo.put("fresh/build/out.o", b"o");
    repo.put("fresh/sub/deep.txt", b"d");
    repo.put("fresh/sub/deep.log", b"d");

    // The walk says only that `fresh` is untracked, with everything in it; its children are told apart by the rules.
    assert_eq!(
        super::tests::marks_with(&repo.path("fresh"), Limits::default()),
        owned(&[
            ("build", Ignored),
            ("debug.log", Ignored),
            ("new.txt", Untracked),
            ("sub", Untracked)
        ])
    );
    assert_eq!(
        super::tests::marks_with(&repo.path("fresh/sub"), Limits::default()),
        owned(&[("deep.log", Ignored), ("deep.txt", Untracked)])
    );
    // From above it is one untracked folder.
    assert_eq!(
        super::tests::marks_with(repo.root(), Limits::default()),
        owned(&[("fresh", Untracked)])
    );
}

#[cfg(unix)]
mod hostile {
    use super::*;
    use crate::view::testing::fifo;

    #[test]
    fn a_nested_head_that_is_a_named_pipe_is_not_opened() {
        let repo = clean(&[("a.txt", b"a")]);
        let nested = nested_git_dir(&repo);
        fifo(&nested, "HEAD");

        assert_sub_is_untracked(in_time(repo.root()), "a pipe for HEAD");
    }

    #[test]
    fn a_nested_git_file_that_points_at_a_folder_whose_head_is_a_named_pipe_is_not_followed() {
        let repo = clean(&[("a.txt", b"a")]);
        let outside = tempfile::tempdir().expect("a folder");
        fs::create_dir_all(outside.path().join("objects")).expect("objects");
        fs::create_dir_all(outside.path().join("refs")).expect("refs");
        fifo(outside.path(), "HEAD");
        fs::create_dir_all(repo.path("sub")).expect("a folder");
        fs::write(repo.path("sub/.git"), format!("gitdir: {}\n", outside.path().display())).expect("a git file");
        fs::write(repo.path("sub/x.txt"), b"x").expect("a file");

        assert_sub_is_untracked(in_time(repo.root()), "a gitdir pointer to a pipe");
    }

    #[test]
    fn a_nested_head_or_commondir_that_is_a_link_to_dev_zero_is_not_read() {
        for (name, others) in [("HEAD", vec![]), ("commondir", vec!["gitdir"])] {
            let repo = clean(&[("a.txt", b"a")]);
            let nested = nested_git_dir(&repo);
            if name != "HEAD" {
                fs::write(nested.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD");
            }
            std::os::unix::fs::symlink("/dev/zero", nested.join(name)).expect("a link");
            for other in others {
                std::os::unix::fs::symlink("/dev/zero", nested.join(other)).expect("a link");
            }

            assert_sub_is_untracked(in_time(repo.root()), name);
        }
    }

    #[test]
    fn a_nested_git_file_that_points_to_a_network_place_or_nowhere_is_not_followed() {
        for pointer in [
            "//host/share/repo.git",
            "/net/host/repo.git",
            "../does-not-exist",
            "gitdir without a path",
        ] {
            let repo = clean(&[("a.txt", b"a")]);
            fs::create_dir_all(repo.path("sub")).expect("a folder");
            fs::write(repo.path("sub/.git"), format!("gitdir: {pointer}\n")).expect("a git file");
            fs::write(repo.path("sub/x.txt"), b"x").expect("a file");

            assert_sub_is_untracked(in_time(repo.root()), pointer);
        }
    }

    #[test]
    fn a_nested_git_folder_with_a_plain_head_gives_the_same_answer() {
        let repo = clean(&[("a.txt", b"a")]);
        let nested = nested_git_dir(&repo);
        fs::write(nested.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD");

        assert_sub_is_untracked(in_time(repo.root()), "a plain nested repository");
    }
}

/// `bytes` of patterns that match nothing in the work tree.
fn rules(bytes: usize) -> Vec<u8> {
    let mut text = Vec::with_capacity(bytes);
    let mut at = 0_u64;
    while text.len() + 17 < bytes {
        text.extend_from_slice(format!("no-such-{at:08x}\n").as_bytes());
        at += 1;
    }

    text
}

#[test]
fn a_rules_file_over_its_cap_keeps_the_walk_out_of_its_folder_and_a_few_small_ones_are_read() {
    let repo = clean(&[("a.txt", b"a")]);
    fs::create_dir_all(repo.path("big")).expect("a folder");
    fs::write(repo.path("big/.gitignore"), rules(300 * 1024)).expect("rules");
    fs::write(repo.path("big/x.txt"), b"x").expect("a file");
    fs::create_dir_all(repo.path("fine")).expect("a folder");
    fs::write(repo.path("fine/.gitignore"), rules(200 * 1024)).expect("rules");
    fs::write(repo.path("fine/x.txt"), b"x").expect("a file");

    // Both are untracked folders; the first is not entered, which does not change what it is.
    assert_eq!(
        super::tests::marks_with(repo.root(), Limits::default()),
        owned(&[("big", Untracked), ("fine", Untracked)])
    );
    // A folder that is browsed itself, with such a file, has no status: the file is read for every entry of it.
    assert_eq!(
        answer(&repo.path("big"), Limits::default()).expect("a status"),
        GitStatus {
            repo: false,
            marks: vec![]
        }
    );
}

#[test]
fn the_rules_files_of_the_folders_entered_add_up_and_past_the_total_the_repository_is_too_big() {
    let repo = clean(&[("a.txt", b"a")]);
    let rules = rules(250 * 1024);
    let make = |count: usize| {
        for folder in 0..count {
            fs::create_dir_all(repo.path(&format!("d{folder:02}"))).expect("a folder");
            fs::write(repo.path(&format!("d{folder:02}/.gitignore")), &rules).expect("rules");
            fs::write(repo.path(&format!("d{folder:02}/x.txt")), b"x").expect("a file");
        }
    };

    // 12 files are 3 MiB: fine. 20 are 5 MiB.
    make(12);
    assert!(answer(repo.root(), Limits::default()).is_ok());
    make(20);
    let too_big = answer(repo.root(), Limits::default());
    assert!(matches!(too_big, Err(GitError::TooBig)), "{too_big:?}");
}

#[test]
fn the_rules_files_of_the_folder_its_parents_and_the_folders_below_it_add_up_too() {
    let repo = clean(&[("a.txt", b"a")]);
    let rules = rules(250 * 1024);
    let mut folder = String::new();
    for level in 0..17 {
        if !folder.is_empty() {
            folder.push('/');
        }
        folder.push_str(&format!("d{level}"));
        fs::create_dir_all(repo.path(&folder)).expect("a folder");
        fs::write(repo.path(&format!("{folder}/.gitignore")), &rules).expect("rules");
    }

    // 17 files of 250 KiB are over 4 MiB.
    let too_big = answer(&repo.path(&folder), Limits::default());
    assert!(matches!(too_big, Err(GitError::TooBig)), "{too_big:?}");
}
