//! The caps of a status (entries, time, the app's exit) and the errors it ends with, which hold no path.

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::tests::{clean, clean_paths};
use super::*;

#[test]
fn more_entries_than_the_cap_is_too_big() {
    let paths: Vec<String> = (0..60).map(|index| format!("dir/file{index:02}.txt")).collect();
    let repo = clean_paths(&paths);

    let small = Limits {
        entries: 30,
        ..Limits::default()
    };
    let answer = status(&repo.path("dir"), small, &AtomicBool::new(false));
    assert!(matches!(answer, Err(GitError::TooBig)), "{answer:?}");

    // The same folder is fine with room.
    assert!(status(&repo.path("dir"), Limits::default(), &AtomicBool::new(false)).is_ok());
}

#[test]
fn a_big_generated_tree_of_untracked_files_is_too_big_under_a_small_cap() {
    let repo = clean(&[("tracked/keep.txt", b"k")]);
    for folder in 0..80 {
        for file in 0..4 {
            repo.put(&format!("tracked/gen{folder:03}/f{file}.txt"), b"x");
        }
    }
    // One tracked file in each makes none of the folders collapse into a single entry.
    let small = Limits {
        entries: 50,
        ..Limits::default()
    };

    let answer = status(&repo.path("tracked"), small, &AtomicBool::new(false));

    assert!(matches!(answer, Err(GitError::TooBig)), "{answer:?}");
}

#[test]
fn the_time_limit_stops_a_status_that_takes_longer() {
    let repo = clean(&[("tracked/keep.txt", b"k")]);
    for folder in 0..300 {
        for file in 0..20 {
            repo.put(&format!("tracked/gen{folder:03}/f{file:02}.txt"), b"x");
        }
    }
    let quick = Limits {
        time: Duration::from_millis(1),
        ..Limits::default()
    };

    let answer = status(&repo.path("tracked"), quick, &AtomicBool::new(false));

    assert!(matches!(answer, Err(GitError::TimedOut)), "{answer:?}");
}

#[test]
fn the_exit_flag_stops_a_status_before_it_starts() {
    let repo = clean(&[("a.txt", b"a")]);

    let answer = status(repo.root(), Limits::default(), &AtomicBool::new(true));

    assert!(matches!(answer, Err(GitError::Cancelled)), "{answer:?}");
}

#[test]
fn no_error_text_holds_a_path() {
    let repo = clean(&[("a.txt", b"a")]);
    let missing = repo.path("not-there");
    let damaged = clean(&[("a.txt", b"a")]);
    fs::write(damaged.git_dir().join("index"), b"DIRC garbage that is no index").expect("a damaged index");
    let seen = [
        status(&missing, Limits::default(), &AtomicBool::new(false)),
        status(damaged.root(), Limits::default(), &AtomicBool::new(false)),
        status(
            repo.root(),
            Limits {
                entries: 0,
                ..Limits::default()
            },
            &AtomicBool::new(false),
        ),
    ];

    let mut texts = Vec::new();
    for answer in seen {
        let Err(error) = answer else {
            panic!("an error was expected");
        };
        texts.push(format!("{error} | {error:?}"));
    }

    assert!(texts[0].contains("no longer exists"), "{}", texts[0]);
    assert!(texts[1].contains("cannot be read"), "{}", texts[1]);
    assert!(texts[2].contains("too big"), "{}", texts[2]);
    for text in &texts {
        for place in [repo.root(), damaged.root(), Path::new("not-there")] {
            assert!(!text.contains(&place.to_string_lossy().into_owned()), "{text}");
        }
    }
}

#[test]
fn every_error_has_a_fixed_text_without_a_path() {
    for error in [
        GitError::TooBig,
        GitError::TimedOut,
        GitError::Cancelled,
        GitError::Folder(std::io::ErrorKind::NotFound),
        GitError::Folder(std::io::ErrorKind::PermissionDenied),
        GitError::Folder(std::io::ErrorKind::Other),
        GitError::Repository,
    ] {
        let text = error.to_string();
        assert!(!text.contains('/') && !text.contains('\\'), "{text}");
    }
}
