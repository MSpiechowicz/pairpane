//! What `columns_git` checks before a thread is taken, the cap on threads, the deadline it answers by, and what the page is
//! told: the answer's shape, and the code and fixed text of each refusal. The status itself is tried in the core crate
//! (`pairpane_core::gitstatus`), against repositories made with gix; here a repository is the few files and folders git
//! needs to see one.

use std::time::{Duration, Instant};

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

/// The status of `path` read in this process (the tests cannot start the app's executable as the helper; that is
/// `tests/git_helper.rs`).
fn git_limited(columns: &Columns, path: &str, limits: Limits) -> CommandResult<GitStatus> {
    block_on(columns.git_with(path.to_owned(), limits, gitstatus::status_in_process))
}

fn git(columns: &Columns, path: &str) -> CommandResult<GitStatus> {
    git_limited(columns, path, Limits::default())
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
                { "name": "fresh", "mark": "untracked" },
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

    let (code, message) = shown(git_limited(&Columns::default(), &path, none));

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

    let (code, message) = shown(git_limited(&Columns::default(), &path, quick));

    // The time limit is `invalidInput` too, with a text of its own.
    assert_eq!(code, IpcErrorCode::InvalidInput);
    assert_eq!(message, "the repository took too long to read");
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

/// What the page is told of a status that ends with `error`.
fn told(error: fn() -> GitError) -> (IpcErrorCode, String) {
    let columns = Columns::default();
    let (_folder, path) = repository();

    shown(block_on(
        columns.git_with(path, Limits::default(), move |_, _, _| Err(error())),
    ))
}

#[test]
fn every_error_of_a_status_has_its_code_and_a_fixed_text() {
    assert_eq!(
        told(|| GitError::TimedOut),
        (
            IpcErrorCode::InvalidInput,
            "the repository took too long to read".to_owned()
        )
    );
    assert_eq!(
        told(|| GitError::TooBig),
        (
            IpcErrorCode::InvalidInput,
            "the repository is too big to read here".to_owned()
        )
    );
    assert_eq!(
        told(|| GitError::Repository),
        (IpcErrorCode::Io, "the repository cannot be read".to_owned())
    );
    assert_eq!(
        told(|| GitError::Cancelled),
        (IpcErrorCode::Io, "the read was cancelled".to_owned())
    );
    assert_eq!(
        told(|| GitError::Folder(io::ErrorKind::PermissionDenied)),
        (IpcErrorCode::Io, "the folder cannot be read: no permission".to_owned())
    );
}

#[test]
fn a_place_is_held_while_a_status_runs_and_given_back_when_it_ends() {
    let (_folder, path) = repository();
    let columns = Columns::default();

    std::thread::scope(|scope| {
        let (started, running) = std::sync::mpsc::channel();
        // Dropped (or sent to) to let the status end; a failed assertion drops it as it unwinds, and so does not hold the
        // scope up for ever.
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let waiting = scope.spawn(|| {
            block_on(columns.git_with(path.clone(), Limits::default(), move |_, _, _| {
                let _ = started.send(());
                let _ = hold.recv();

                Ok(GitStatus {
                    repo: false,
                    marks: Vec::new(),
                })
            }))
        });
        running.recv_timeout(Duration::from_secs(10)).expect("the status runs");

        // Two places, one of them held: the other is free, and with both held a status is refused.
        let taken = columns.statuses.acquire(GIT_THREADS).expect("the other place");
        assert_eq!(shown(git(&columns, &path)).0, IpcErrorCode::Busy);
        drop(taken);
        assert!(git(&columns, &path).expect("the other place").repo);

        drop(release);
        assert!(!waiting.join().expect("the thread").expect("a status").repo);
    });

    assert_eq!(
        fill(&columns.statuses, GIT_THREADS).len(),
        GIT_THREADS,
        "every place is back"
    );
}

#[test]
fn a_folder_that_cannot_be_resolved_is_not_found_or_io_and_never_invalid_input() {
    let folder = tempfile::tempdir().expect("a folder");
    let missing = folder.path().join("gone").join("deeper").to_string_lossy().into_owned();

    assert_eq!(shown(git(&Columns::default(), &missing)).0, IpcErrorCode::NotFound);

    #[cfg(unix)]
    {
        // A link that leads to itself has no real path.
        let looped = folder.path().join("loop");
        std::os::unix::fs::symlink(&looped, &looped).expect("a link");

        let (code, message) = shown(git(&Columns::default(), &looped.to_string_lossy()));

        assert_eq!(code, IpcErrorCode::Io);
        assert_eq!(message, "the folder cannot be read");
    }
}

/// Unix: a named pipe where `HEAD` should be, which a plain open of waits on for ever.
#[cfg(unix)]
#[test]
fn a_repository_with_a_named_pipe_for_head_is_not_a_repo_and_the_answer_comes() {
    let (folder, path) = repository();
    let head = folder.path().join(".git").join("HEAD");
    std::fs::remove_file(&head).expect("removed");
    let status = std::process::Command::new("mkfifo")
        .arg(&head)
        .status()
        .expect("mkfifo runs");
    assert!(status.success());

    let started = Instant::now();
    let answer = git(&Columns::default(), &path).expect("a status");

    assert!(!answer.repo);
    assert!(started.elapsed() < Duration::from_secs(5));
}
