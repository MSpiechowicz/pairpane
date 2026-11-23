//! What `columns_git` checks before a thread is taken, the cap on threads, and what the page is told: the answer's shape,
//! and the code and fixed text of each refusal. The status itself is tried in the core crate (`pairpane_core::gitstatus`),
//! against repositories made with gix; here a repository is the few files and folders git needs to see one.

use std::time::Duration;

use serde_json::json;
use tauri::async_runtime::block_on;

use super::tests::{fill, shown};
use super::*;

/// A folder that is the work tree of a repository with no commit: `HEAD`, `config`, `objects` and `refs`.
fn repository() -> (tempfile::TempDir, String) {
    let folder = tempfile::tempdir().expect("a folder");
    let git = folder.path().join(".git");
    std::fs::create_dir_all(git.join("objects")).expect("objects");
    std::fs::create_dir_all(git.join("refs")).expect("refs");
    std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD");
    std::fs::write(
        git.join("config"),
        "[core]\n\trepositoryformatversion = 0\n\tbare = false\n",
    )
    .expect("config");
    let path = folder.path().to_string_lossy().into_owned();

    (folder, path)
}

fn git(columns: &Columns, path: &str) -> CommandResult<GitStatus> {
    block_on(columns.git(path.to_owned(), Limits::default()))
}

#[test]
fn a_folder_of_a_repository_is_told_with_the_marks_of_its_children_as_the_page_reads_them() {
    let (folder, path) = repository();
    std::fs::write(folder.path().join("new.txt"), b"n").expect("a file");
    std::fs::create_dir(folder.path().join("fresh")).expect("a folder");
    std::fs::write(folder.path().join("fresh").join("inner.txt"), b"i").expect("a file");

    let answer = git(&Columns::default(), &path).expect("a status");

    assert_eq!(
        serde_json::to_value(&answer).expect("json"),
        json!({
            "repo": true,
            "marks": [
                { "name": "fresh", "mark": "modified" },
                { "name": "new.txt", "mark": "untracked" },
            ],
        })
    );
}

#[test]
fn a_folder_that_is_in_no_repository_is_not_an_error() {
    let folder = tempfile::tempdir().expect("a folder");
    std::fs::write(folder.path().join("a.txt"), b"a").expect("a file");
    let path = folder.path().to_string_lossy().into_owned();

    let answer = git(&Columns::default(), &path).expect("a status");

    assert_eq!(
        serde_json::to_value(&answer).expect("json"),
        json!({ "repo": false, "marks": [] })
    );
}

#[test]
fn what_the_page_sent_is_checked_before_a_thread_is_taken_and_a_full_cap_is_busy() {
    let (_folder, path) = repository();
    let columns = Columns::default();
    let taken = fill(&columns.statuses, GIT_THREADS);

    assert_eq!(shown(git(&columns, "relative/folder")).0, IpcErrorCode::InvalidInput);
    assert_eq!(shown(git(&columns, &path)).0, IpcErrorCode::Busy);

    drop(taken);
    assert!(git(&columns, &path).expect("a freed place is used").repo);
}

#[test]
fn a_repository_over_the_cap_is_invalid_input_with_a_fixed_text_and_no_path() {
    let (folder, path) = repository();
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(folder.path().join(name), b"x").expect("a file");
    }
    let none = Limits {
        entries: 0,
        ..Limits::default()
    };

    let (code, message) = shown(block_on(Columns::default().git(path.clone(), none)));

    assert_eq!(code, IpcErrorCode::InvalidInput);
    assert_eq!(message, "the repository is too big to read here");
    assert!(!message.contains(&path));
}

#[test]
fn a_status_that_takes_longer_than_its_time_is_the_same_answer() {
    let (folder, path) = repository();
    for index in 0..400 {
        let sub = folder.path().join(format!("gen{index:03}"));
        std::fs::create_dir(&sub).expect("a folder");
        for file in 0..8 {
            std::fs::write(sub.join(format!("f{file}.txt")), b"x").expect("a file");
        }
        // A file of its own at the top makes the folders not collapse into one entry of the walk.
        std::fs::write(folder.path().join(format!("top{index:03}.txt")), b"x").expect("a file");
    }
    let quick = Limits {
        time: Duration::from_millis(1),
        ..Limits::default()
    };

    let (code, message) = shown(block_on(Columns::default().git(path, quick)));

    assert_eq!(code, IpcErrorCode::InvalidInput);
    assert_eq!(message, "the repository is too big to read here");
}

#[test]
fn a_folder_that_is_gone_is_not_found_with_a_text_that_has_no_path() {
    let (folder, _) = repository();
    let gone = folder.path().join("missing").to_string_lossy().into_owned();

    let (code, message) = shown(git(&Columns::default(), &gone));

    assert_eq!(code, IpcErrorCode::NotFound);
    assert_eq!(message, "the folder no longer exists");
    assert!(!message.contains(&gone));
}

#[test]
fn the_exit_flag_refuses_a_status_that_comes_after_it() {
    let (_folder, path) = repository();
    let columns = Columns::default();
    columns.shut_down();

    let (code, message) = shown(git(&columns, &path));

    assert_eq!(code, IpcErrorCode::Io);
    assert_eq!(message, "the read was cancelled");
}
