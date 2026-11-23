//! The rules budget (see [`super::budget`]): every source of ignore or attribute rules a status may read is counted before gix
//! reads it, in one budget, whether it is a file of the work tree or the blob of an index entry that stands in for one. These
//! are the shapes the audits found that nothing counted: a chain of folders that each hold a big `.gitattributes` or
//! `.gitignore` in the index only (deleted from the work tree, or marked skip-worktree), and one in the work tree at every
//! level of a chain of folders that a changed file is below. Each is `TooBig`, quickly.

use std::fs;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant, SystemTime};

use super::fixture::{Repo, Staged, pairs};
use super::testing::status;
use super::*;
use crate::model::GitMark::{Ignored, Untracked};

/// Rules of about `bytes` bytes that say nothing that matches (`a x`, an attribute line).
fn rules(bytes: usize) -> Vec<u8> {
    "a x\n".repeat(bytes / 4).into_bytes()
}

/// How many levels of folders: each holds 250 KiB, so 34 of them are over 8 MiB.
const LEVELS: usize = 40;

/// A repository of a chain of `LEVELS` folders `d/d/d/...` that each hold a tracked `name` of 250 KiB in the index, the files
/// of which are in the work tree (`in_tree`) or not, with a tracked `f.txt` at the bottom that has another content than the
/// index has. Returns the repository and the folder at the bottom.
fn chain(name: &str, in_tree: bool, skipped: bool) -> (Repo, String) {
    let repo = Repo::new();
    let body = rules(250 * 1024);
    let mut folder = String::new();
    let mut paths = Vec::new();

    for _ in 0..LEVELS {
        paths.push(format!("{folder}{name}"));
        folder.push_str("d/");
    }
    fs::create_dir_all(repo.path(&folder)).expect("folders");
    paths.push(format!("{folder}f.txt"));

    // The index sorts by path: `d/.gitattributes` comes before `d/d/...`.
    paths.sort();
    let contents: Vec<&[u8]> = paths
        .iter()
        .map(|path| {
            if path.ends_with("f.txt") {
                b"hello".as_slice()
            } else {
                body.as_slice()
            }
        })
        .collect();
    let entries: Vec<Staged<'_>> = paths
        .iter()
        .zip(&contents)
        .map(|(path, content)| {
            let entry = Staged::file(path, content);

            if skipped && !path.ends_with("f.txt") {
                entry.skipped()
            } else {
                entry
            }
        })
        .collect();

    if in_tree {
        // As `git add` leaves them: the stat of each file is the index's, so none is read to be compared (reading is the slow
        // part of a status, and these are not what is looked at) but the one file that is changed below.
        for (path, content) in paths.iter().zip(&contents) {
            repo.put(path, content);
        }
        repo.write_index_with_disk_stat(&entries);
    } else {
        repo.write_index(&entries);
    }
    // Same size, another content and a later time: compared by content, through the attributes of every folder above it.
    repo.put(&format!("{folder}f.txt"), b"hellx");
    let later = SystemTime::now() + Duration::from_secs(60);
    fs::File::options()
        .write(true)
        .open(repo.path(&format!("{folder}f.txt")))
        .and_then(|file| file.set_modified(later))
        .expect("a later time");

    (repo, folder)
}

/// The status of the root is `TooBig`, and not for the time it took: it has more time than a loaded machine needs (these
/// are debug builds, run in parallel with the rest), and is far from using it when it is counted as it should be.
fn quick_too_big(repo: &Repo) {
    let limits = Limits {
        time: Duration::from_secs(40),
        ..Limits::default()
    };
    let started = Instant::now();
    let answer = status(repo.root(), limits, &AtomicBool::new(false));

    assert!(matches!(answer, Err(GitError::TooBig)), "{answer:?}");
    assert!(started.elapsed() < Duration::from_secs(30), "{:?}", started.elapsed());
}

#[test]
fn attributes_in_the_index_only_at_every_level_of_a_chain_are_counted_and_too_big() {
    let (repo, _) = chain(".gitattributes", false, false);

    quick_too_big(&repo);
}

#[test]
fn ignore_rules_marked_skip_worktree_at_every_level_of_a_chain_are_counted_and_too_big() {
    let (repo, folder) = chain(".gitignore", false, true);
    repo.put(&format!("{folder}new.txt"), b"n");

    quick_too_big(&repo);
}

#[test]
fn attributes_in_the_work_tree_at_every_level_of_a_chain_are_counted_and_too_big() {
    let (repo, _) = chain(".gitattributes", true, false);

    quick_too_big(&repo);
}

#[test]
fn a_chain_of_rules_that_adds_up_to_less_than_the_total_is_read() {
    let repo = Repo::new();
    let body = rules(100 * 1024);
    repo.commit(&[
        ("d/.gitattributes", &body),
        ("d/e/.gitattributes", &body),
        ("d/e/f.txt", b"hello"),
    ]);
    repo.put("d/e/f.txt", b"hellx");

    let answer = status(repo.root(), Limits::default(), &AtomicBool::new(false)).expect("a status");

    assert_eq!(pairs(&answer.marks), vec![("d", crate::model::GitMark::Modified)]);
}

#[test]
fn a_blob_over_the_size_of_a_rules_file_on_the_chain_of_the_folder_is_too_big() {
    let repo = Repo::new();
    repo.write_index(&[
        Staged::file(".gitattributes", &rules(300 * 1024)),
        Staged::file("d/a.txt", b"a"),
    ]);
    repo.put("d/a.txt", b"a");

    let answer = status(&repo.path("d"), Limits::default(), &AtomicBool::new(false));

    assert!(matches!(answer, Err(GitError::TooBig)), "{answer:?}");
}

#[test]
fn a_subfolder_of_an_untracked_folder_that_holds_only_ignored_files_is_ignored_as_git_has_it() {
    let repo = Repo::new();
    repo.commit(&[(".gitignore", b"*.o\n"), ("a.txt", b"a")]);
    for (path, content) in [
        ("build/only/x.o", "x"),
        ("build/deep/inner/q.o", "q"),
        ("build/deep/inner/r.o", "r"),
        ("build/mixed/a.o", "a"),
        ("build/mixed/b.txt", "b"),
        ("build/nested/v.o", "v"),
        ("build/nested/.git/HEAD", "ref: refs/heads/main\n"),
        ("build/top.o", "t"),
        ("build/top.txt", "t"),
    ] {
        repo.put(path, content.as_bytes());
    }
    fs::create_dir_all(repo.path("build/empty")).expect("a folder");

    let answer = status(&repo.path("build"), Limits::default(), &AtomicBool::new(false)).expect("a status");

    assert_eq!(
        pairs(&answer.marks),
        vec![
            ("deep", Ignored),
            ("empty", Untracked),
            ("mixed", Untracked),
            ("nested", Untracked),
            ("only", Ignored),
            ("top.o", Ignored),
            ("top.txt", Untracked),
        ]
    );
}

#[test]
fn the_look_into_the_subfolders_of_an_untracked_folder_is_bounded() {
    let repo = Repo::new();
    repo.commit(&[(".gitignore", b"*.o\n"), ("a.txt", b"a")]);
    // More ignored files than the look at them takes in (`ignored::LOOKED_AT`): the folder is untracked, which is what is
    // said when nothing is known, and the status is no error.
    fs::create_dir_all(repo.path("build/many")).expect("a folder");
    repo.put("build/top.txt", b"t");
    for index in 0..10_500 {
        fs::write(repo.path(&format!("build/many/{index}.o")), b"").expect("a file");
    }

    let answer = status(&repo.path("build"), Limits::default(), &AtomicBool::new(false)).expect("a status");

    assert_eq!(pairs(&answer.marks), vec![("many", Untracked), ("top.txt", Untracked)]);
}
