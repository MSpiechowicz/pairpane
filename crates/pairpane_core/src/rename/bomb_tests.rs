//! A rule made to grow a name without bound (templates that repeat the name, a search that matches everywhere) is
//! answered at once, with names that are too long.

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::testing::rule;
use super::*;

/// How long the preview of the bomb may take: it takes milliseconds, and without the limits minutes.
const PATIENCE: Duration = Duration::from_secs(5);

fn bomb() -> RenameRule {
    RenameRule {
        name: "[N]".repeat(341),
        ext: "[N]".repeat(341),
        search: "x?".to_owned(),
        replace: "z".repeat(1024),
        regex: true,
        ..rule()
    }
}

#[test]
fn a_rule_that_grows_the_name_without_bound_gives_names_that_are_too_long_and_is_quick() {
    let folder = tempfile::tempdir().expect("temp dir");
    let names: Vec<String> = (0..10).map(|index| format!("{}{index:03}", "a".repeat(240))).collect();
    for name in &names {
        std::fs::write(folder.path().join(name), b"").expect("file");
    }

    let path = folder.path().to_path_buf();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let started = Instant::now();
        let preview = preview_in(&path, &names, &bomb());
        let _ = sender.send((started.elapsed(), preview));
    });
    let (elapsed, preview) = receiver.recv_timeout(PATIENCE).expect("the preview ends in time");

    let RenamePreview::Rows { rows, problems, .. } = preview else {
        panic!("a rule that works");
    };
    eprintln!("bomb preview of {} names took {elapsed:?}", rows.len());
    assert_eq!(rows.len(), 10);
    assert_eq!(problems, 10);
    for row in &rows {
        assert_eq!(row.problem, Some(RenameProblem::TooLong));
        assert!(row.to.len() <= MAX_NAME_BYTES, "{}", row.to.len());
    }
}

#[test]
fn a_replacement_that_makes_the_name_long_is_cut_and_too_long() {
    let folder = tempfile::tempdir().expect("temp dir");
    std::fs::write(folder.path().join("abc"), b"").expect("file");
    let rule = RenameRule {
        search: "b".to_owned(),
        replace: "z".repeat(1024),
        ..rule()
    };

    let RenamePreview::Rows { rows, .. } = preview_in(folder.path(), &["abc".to_owned()], &rule) else {
        panic!("a rule that works");
    };

    assert_eq!(rows[0].problem, Some(RenameProblem::TooLong));
    assert!(rows[0].to.len() <= MAX_NAME_BYTES);
}
