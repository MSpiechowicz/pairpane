//! The walk: what it visits, what it prunes, what it never follows, and how it stops.

use std::fs;

use pretty_assertions::assert_eq;

use crate::model::{EntryKind, FindEvent, FindHit, Tri};

use super::testing::{folder, named, put, search, search_limited, search_stopped, temp, try_search};
use super::{Limits, entry};

fn sample() -> tempfile::TempDir {
    let root = temp();
    put(root.path(), "a.txt", b"a");
    put(root.path(), "docs/b.txt", b"b");
    put(root.path(), "docs/deep/c.TXT", b"c");
    put(root.path(), "docs/deep/d.md", b"d");
    folder(root.path(), "empty");

    root
}

#[test]
fn a_mask_finds_files_at_any_depth_by_their_path_below_the_folder() {
    let root = sample();

    let found = search(root.path(), named("*.txt"));

    assert_eq!(found.paths(), ["a.txt", "docs/b.txt", "docs/deep/c.TXT"]);
    assert_eq!(found.done(), (3, 0, false, false));
}

#[test]
fn a_blank_name_finds_everything_and_the_folder_itself_is_not_a_result() {
    let root = sample();

    let found = search(root.path(), named("  "));

    assert_eq!(
        found.paths(),
        [
            "a.txt",
            "docs",
            "docs/b.txt",
            "docs/deep",
            "docs/deep/c.TXT",
            "docs/deep/d.md",
            "empty"
        ]
    );
}

#[test]
fn results_say_what_they_are() {
    let root = sample();

    let found = search(root.path(), named("docs;b.txt"));
    let hits = found.hits();

    assert_eq!(hits.len(), 2);
    assert_eq!(
        (hits[0].path.as_str(), hits[0].kind, hits[0].size),
        ("docs", EntryKind::Dir, 0)
    );
    assert_eq!(
        (hits[1].path.as_str(), hits[1].kind, hits[1].size),
        ("docs/b.txt", EntryKind::File, 1)
    );
    assert!(hits[1].modified.is_some());
    assert!(
        hits.iter()
            .all(|hit| !hit.link && hit.inner.is_none() && hit.matches.is_none())
    );
}

#[test]
fn a_mask_can_be_tested_on_the_whole_path() {
    let root = sample();
    let mut criteria = named("docs/*.txt");
    criteria.name_path = true;
    criteria.name_case = true;

    let found = search(root.path(), criteria);

    assert_eq!(found.paths(), ["docs/b.txt"]);
}

#[test]
fn a_regular_expression_is_tested_on_the_name_with_or_without_case() {
    let root = sample();
    let mut criteria = named(r"^[bc]\.txt$");
    criteria.name_regex = true;

    assert_eq!(
        search(root.path(), criteria.clone()).paths(),
        ["docs/b.txt", "docs/deep/c.TXT"]
    );

    criteria.name_case = true;
    assert_eq!(search(root.path(), criteria).paths(), ["docs/b.txt"]);
}

#[test]
fn hidden_files_are_found_unless_skipped_and_a_hidden_folder_is_not_entered() {
    if cfg!(windows) {
        return;
    }
    let root = temp();
    put(root.path(), ".hidden.txt", b"x");
    put(root.path(), ".cache/inside.txt", b"x");
    put(root.path(), "shown.txt", b"x");

    let mut criteria = named("*.txt");
    assert_eq!(
        search(root.path(), criteria.clone()).paths(),
        [".cache/inside.txt", ".hidden.txt", "shown.txt"]
    );

    criteria.skip_hidden = true;
    assert_eq!(search(root.path(), criteria.clone()).paths(), ["shown.txt"]);

    criteria.skip_hidden = false;
    criteria.hidden = Tri::Yes;
    assert_eq!(search(root.path(), criteria).paths(), [".hidden.txt"]);
}

#[test]
fn ignore_files_are_read_only_when_asked_to() {
    let root = temp();
    put(root.path(), ".gitignore", b"target/\n*.log\n");
    put(root.path(), ".git/HEAD", b"ref: refs/heads/main\n");
    put(root.path(), "src/main.rs", b"x");
    put(root.path(), "target/out.rs", b"x");
    put(root.path(), "src/trace.log", b"x");
    let mut criteria = named("*.rs;*.log");

    assert_eq!(
        search(root.path(), criteria.clone()).paths(),
        ["src/main.rs", "src/trace.log", "target/out.rs"]
    );

    criteria.skip_ignored = true;
    assert_eq!(search(root.path(), criteria).paths(), ["src/main.rs"]);
}

#[test]
fn a_folder_that_does_not_exist_or_is_a_file_cannot_be_searched() {
    let root = sample();

    assert!(try_search(&root.path().join("missing"), named("*")).is_err());
    assert!(try_search(&root.path().join("a.txt"), named("*")).is_err());
    assert!(try_search(std::path::Path::new("relative"), named("*")).is_err());
}

#[test]
fn progress_counts_what_was_looked_at() {
    let root = sample();

    let found = search(root.path(), named("nothing-fits"));

    assert!(found.paths().is_empty());
    assert!(found.events.iter().any(|event| matches!(
        event,
        FindEvent::Progress {
            folders: 3,
            files: 4,
            bytes: 4
        }
    )));
}

#[test]
fn the_search_stops_at_the_cap_and_says_so() {
    let root = temp();
    for index in 0..50 {
        put(root.path(), &format!("f{index}.txt"), b"x");
    }

    let limits = Limits {
        hits: 5,
        ..Limits::DEFAULT
    };
    let found = search_limited(root.path(), named("*.txt"), limits);

    assert_eq!(found.paths().len(), 5);
    assert_eq!(found.done(), (5, 0, true, false));
}

#[test]
fn a_cancelled_search_ends_with_done_and_says_it_was_cancelled() {
    let root = temp();
    for index in 0..400 {
        put(root.path(), &format!("d{}/f{index}.txt", index % 7), b"x");
    }

    let before = search_stopped(root.path(), named("*"), true, |_| false);
    assert!(before.done().3);

    let during = search_stopped(root.path(), named("*"), false, |event| {
        matches!(event, FindEvent::Found { .. })
    });
    assert!(during.done().3);
}

#[test]
fn results_come_in_batches_of_at_most_256() {
    let root = temp();
    for index in 0..700 {
        put(root.path(), &format!("f{index}.txt"), b"x");
    }

    let found = search(root.path(), named("*.txt"));

    assert_eq!(found.paths().len(), 700);
    for event in &found.events {
        if let FindEvent::Found { hits } = event {
            assert!(hits.len() <= 256, "{}", hits.len());
        }
    }
}

#[test]
fn a_result_is_read_as_a_pane_lists_it_with_its_path_as_its_name() {
    let root = sample();
    let found = search(root.path(), named("c.txt"));
    let hit = &found.hits()[0];

    let listed = entry(root.path(), hit).expect("the file is there");

    assert_eq!(listed.name, "docs/deep/c.TXT");
    assert_eq!((listed.kind, listed.size), (EntryKind::File, 1));

    fs::remove_file(root.path().join("docs/deep/c.TXT")).expect("remove");
    assert_eq!(entry(root.path(), hit), None);
}

#[test]
fn a_result_with_a_path_that_climbs_or_inside_an_archive_is_not_read() {
    let root = sample();
    let hit = |path: &str, inner: Option<&str>| FindHit {
        path: path.to_owned(),
        inner: inner.map(str::to_owned),
        kind: EntryKind::File,
        link: false,
        size: 0,
        modified: None,
        matches: None,
        line: None,
        line_number: None,
    };

    assert_eq!(entry(root.path(), &hit("../x", None)), None);
    assert_eq!(entry(root.path(), &hit("docs//b.txt", None)), None);
    assert_eq!(entry(root.path(), &hit("a.txt", Some("inner"))), None);
}

// A name is bytes on Linux: `x\xFF` and `x\u{FFFD}` are two files, and a lossy path would read the second for the first.
#[cfg(target_os = "linux")]
#[test]
fn a_name_that_is_not_valid_unicode_is_skipped_and_cannot_stand_for_another_file() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let root = temp();
    fs::write(root.path().join(OsStr::from_bytes(b"x\xFF")), b"needle in the odd one").expect("write");
    put(root.path(), "x\u{FFFD}", b"nothing here");
    fs::create_dir(root.path().join(OsStr::from_bytes(b"d\xFE"))).expect("folder");
    fs::write(root.path().join(OsStr::from_bytes(b"d\xFE")).join("in.txt"), b"needle").expect("write");
    put(root.path(), "plain.txt", b"needle");
    let mut criteria = named("*");
    criteria.text = Some(super::content_tests::text("needle"));

    let by_content = search(root.path(), criteria);
    let by_name = search(root.path(), named("*"));

    assert_eq!(by_content.paths(), ["plain.txt"]);
    assert_eq!(by_content.done().1, 2);
    assert_eq!(by_name.paths(), ["plain.txt", "x\u{FFFD}"]);
    assert_eq!(by_name.done().1, 2);
}

#[cfg(unix)]
mod links {
    use std::os::unix::fs::symlink;

    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn a_link_is_a_result_and_is_never_entered() {
        let root = temp();
        put(root.path(), "real/inside.txt", b"x");
        symlink(root.path().join("real"), root.path().join("to-real")).expect("link");
        symlink(root.path().join("real/inside.txt"), root.path().join("to-file.txt")).expect("link");

        let found = search(root.path(), named("*.txt;to-*"));
        let hits = found.hits();

        assert_eq!(found.paths(), ["real/inside.txt", "to-file.txt", "to-real"]);
        assert!(hits[1].link && hits[2].link);
        assert!(hits[1..].iter().all(|hit| hit.kind == EntryKind::Other));
        assert!(!hits[0].link);
    }

    #[test]
    fn a_result_in_a_folder_that_was_swapped_for_a_link_after_the_search_is_not_read() {
        let root = sample();
        let found = search(root.path(), named("c.txt"));
        let hit = &found.hits()[0];
        assert!(entry(root.path(), hit).is_some());

        let elsewhere = temp();
        put(elsewhere.path(), "deep/c.TXT", b"another file");
        fs::remove_dir_all(root.path().join("docs")).expect("remove");
        symlink(elsewhere.path(), root.path().join("docs")).expect("link");

        assert!(
            root.path().join("docs/deep/c.TXT").exists(),
            "the path now leads to another file"
        );
        assert_eq!(entry(root.path(), hit), None);
    }

    #[test]
    fn a_result_below_a_folder_that_became_a_file_is_not_read() {
        let root = sample();
        let found = search(root.path(), named("c.txt"));
        let hit = &found.hits()[0];

        fs::remove_dir_all(root.path().join("docs")).expect("remove");
        put(root.path(), "docs", b"a file now");

        assert_eq!(entry(root.path(), hit), None);
    }

    #[test]
    fn a_link_that_loops_back_does_not_loop_the_search() {
        let root = temp();
        put(root.path(), "a/b/file.txt", b"x");
        symlink(root.path(), root.path().join("a/b/up")).expect("link");

        let found = search(root.path(), named("*.txt"));

        assert_eq!(found.paths(), ["a/b/file.txt"]);
    }

    #[test]
    fn a_folder_that_is_itself_a_link_is_searched_below() {
        let real = temp();
        put(real.path(), "in.txt", b"x");
        let holder = temp();
        let link = holder.path().join("link");
        symlink(real.path(), &link).expect("link");

        assert_eq!(search(&link, named("*.txt")).paths(), ["in.txt"]);
    }

    #[test]
    fn a_pipe_is_listed_by_name_and_never_opened() {
        let root = temp();
        crate::view::testing::fifo(root.path(), "pipe");
        put(root.path(), "plain.txt", b"needle");
        let mut criteria = named("*");
        criteria.text = Some(super::super::content_tests::text("needle"));

        let found = search(root.path(), criteria);

        assert_eq!(found.paths(), ["plain.txt"]);
        let by_name = search(root.path(), named("pipe"));
        assert_eq!(by_name.hits()[0].kind, EntryKind::Other);
    }

    // A path of 2 200 bytes is too long for macOS (1 024) and Windows (260), where this cannot be built.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_depth_is_bounded() {
        let root = temp();
        let mut below = root.path().to_owned();
        for _ in 0..1_100 {
            below.push("d");
        }
        fs::create_dir_all(&below).expect("a deep tree");
        fs::write(below.join("deep.txt"), b"x").expect("write");
        put(root.path(), "d/d/shallow.txt", b"x");

        let found = search(root.path(), named("*.txt"));

        assert_eq!(found.paths(), ["d/d/shallow.txt"]);
        assert_eq!(found.done().1, 0);
    }
}
