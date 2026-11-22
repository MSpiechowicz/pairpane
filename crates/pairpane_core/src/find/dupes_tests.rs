//! Duplicates: same size, then the same start, then the same content.

use pretty_assertions::assert_eq;

use crate::model::{FindCriteria, FindEvent};

use super::testing::{put, search, search_limited, search_stopped, temp};
use super::{HASH_HEAD, Limits};

fn duplicates() -> FindCriteria {
    FindCriteria {
        duplicates: true,
        ..FindCriteria::default()
    }
}

fn big(tail: u8) -> Vec<u8> {
    let mut bytes = vec![7_u8; HASH_HEAD as usize * 2];
    bytes.push(tail);

    bytes
}

#[test]
fn files_of_the_same_content_make_a_group_and_the_others_do_not() {
    let root = temp();
    put(root.path(), "a/one.txt", b"same text");
    put(root.path(), "b/two.txt", b"same text");
    put(root.path(), "c/three.txt", b"same text");
    put(root.path(), "d/other.txt", b"diff text");
    put(root.path(), "e/alone.txt", b"unique");

    let found = search(root.path(), duplicates());

    assert_eq!(found.groups(), [["a/one.txt", "b/two.txt", "c/three.txt"]]);
    assert_eq!(found.done(), (3, 0, false, false));
    assert!(
        found.paths().is_empty(),
        "duplicates come as groups, not as found events"
    );
}

#[test]
fn a_group_says_the_size_of_its_files() {
    let root = temp();
    put(root.path(), "a", b"12345");
    put(root.path(), "b", b"12345");

    let found = search(root.path(), duplicates());

    assert!(
        found
            .events
            .iter()
            .any(|event| matches!(event, FindEvent::Group { size: 5, hits } if hits.len() == 2))
    );
}

#[test]
fn empty_files_are_not_duplicates_of_each_other() {
    let root = temp();
    put(root.path(), "a", b"");
    put(root.path(), "b", b"");

    let found = search(root.path(), duplicates());

    assert!(found.groups().is_empty());
}

#[test]
fn files_that_differ_after_the_first_block_are_told_apart() {
    let root = temp();
    put(root.path(), "x1", &big(1));
    put(root.path(), "x2", &big(1));
    put(root.path(), "y", &big(2));
    let mut other_start = big(1);
    other_start[0] = 8;
    put(root.path(), "z", &other_start);

    let found = search(root.path(), duplicates());

    assert_eq!(found.groups(), [["x1", "x2"]]);
    // The start of the four; then all of the three with the same start, which only the end tells apart.
    let size = big(1).len() as u64;
    assert_eq!(found.bytes(), 4 * HASH_HEAD + 3 * size);
}

#[test]
fn a_small_file_is_hashed_once_whole() {
    let root = temp();
    put(root.path(), "a", b"abc");
    put(root.path(), "b", b"abc");

    assert_eq!(search(root.path(), duplicates()).bytes(), 6);
}

#[test]
fn the_name_and_other_tests_narrow_the_candidates() {
    let root = temp();
    put(root.path(), "a.jpg", b"pixels");
    put(root.path(), "b.jpg", b"pixels");
    put(root.path(), "a.txt", b"pixels");
    let mut criteria = duplicates();
    criteria.names = "*.jpg".to_owned();

    assert_eq!(search(root.path(), criteria).groups(), [["a.jpg", "b.jpg"]]);
}

#[cfg(unix)]
#[test]
fn hard_links_are_one_file_and_links_are_not_files_of_their_own() {
    let root = temp();
    put(root.path(), "orig", b"content");
    std::fs::hard_link(root.path().join("orig"), root.path().join("hard")).expect("hard link");
    std::os::unix::fs::symlink(root.path().join("orig"), root.path().join("soft")).expect("link");

    assert!(search(root.path(), duplicates()).groups().is_empty());

    put(root.path(), "copy", b"content");
    let found = search(root.path(), duplicates());

    assert_eq!(found.groups().len(), 1);
    assert_eq!(found.groups()[0].len(), 2);
    assert!(found.groups()[0].contains(&"copy".to_owned()));
    assert_eq!(found.done().0, 2);
}

#[test]
fn too_many_candidates_stop_the_walk_and_say_so() {
    let root = temp();
    for index in 0..20 {
        put(root.path(), &format!("f{index}"), b"same");
    }

    let limits = Limits {
        candidates: 4,
        ..Limits::DEFAULT
    };
    let found = search_limited(root.path(), duplicates(), limits);

    assert!(found.done().2);
    assert!(found.groups().iter().all(|group| group.len() <= 4));
}

#[test]
fn groups_beyond_the_cap_on_results_are_not_sent() {
    let root = temp();
    for (name, content) in [("a1", "aaa"), ("a2", "aaa"), ("b1", "bbbb"), ("b2", "bbbb")] {
        put(root.path(), name, content.as_bytes());
    }

    let limits = Limits {
        hits: 3,
        ..Limits::DEFAULT
    };
    let found = search_limited(root.path(), duplicates(), limits);

    assert_eq!(found.groups().len(), 1);
    assert_eq!(found.done(), (2, 0, true, false));
}

#[test]
fn a_stop_ends_the_search_before_or_during_the_hashing() {
    let root = temp();
    for index in 0..30 {
        let content = format!("{index:04}").repeat(index + 1);
        put(root.path(), &format!("a{index}"), content.as_bytes());
        put(root.path(), &format!("b{index}"), content.as_bytes());
    }

    let before = search_stopped(root.path(), duplicates(), true, |_| false);
    assert!(before.done().3);
    assert!(before.groups().is_empty());

    let during = search_stopped(root.path(), duplicates(), false, |event| {
        matches!(event, FindEvent::Group { .. })
    });
    assert!(during.done().3);
    assert!(!during.groups().is_empty());
}
