//! Synchronize jobs whose target changes while an item is being copied: what a job decides per item, at the moment it
//! writes, so nothing the user put there since the scan is replaced.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pretty_assertions::assert_eq;

use crate::model::{OpsEvent, SyncDirection::ToRight};

use super::hooks::Hooks;
use super::sync_tests::{Pair, file, is_done, moment, sync_of};
use super::testing::{Rig, has_no_partial_files, modified, read, set_times, tree};

/// A source big enough to be copied in several chunks, so an event of the job comes before it is committed.
const BIG: usize = 8 * 1024 * 1024;

/// A rig that calls `act` once, on the first progress event with bytes written (the job is then in the middle of a file).
fn rig_acting_once(act: impl Fn() + Send + Sync + 'static) -> Rig {
    let hooks = Hooks {
        every_chunk: true,
        buffered_only: true,
        ..Hooks::default()
    };
    let done = AtomicBool::new(false);

    Rig::with(hooks, move |_, event| {
        if let OpsEvent::Progress { done_bytes: 1.., .. } = event
            && !done.swap(true, Ordering::SeqCst)
        {
            act();
        }
    })
}

/// A newer `doc.bin` on the left (big) and an older one on the right, and the job that copies it to the right.
fn replacing_old() -> (Pair, super::Request) {
    let pair = Pair::new();
    file(&pair.left().join("doc.bin"), &vec![7; BIG], 300);
    file(&pair.right().join("doc.bin"), b"old", 200);
    let request = sync_of(&pair.left(), &pair.right(), &["doc.bin"], ToRight);

    (pair, request)
}

#[test]
fn a_file_that_appears_at_a_free_target_during_the_copy_is_kept() {
    let pair = Pair::new();
    file(&pair.left().join("new.bin"), &vec![7; BIG], 100);
    let request = sync_of(&pair.left(), &pair.right(), &["new.bin"], ToRight);
    let appearing = pair.right().join("new.bin");

    let ended = rig_acting_once(writes(appearing, b"user data written meanwhile")).run(request);

    is_done(&ended, 1);
    assert_eq!(read(&pair.right().join("new.bin")), b"user data written meanwhile");
    assert!(has_no_partial_files(pair.root.path()));
}

#[test]
fn a_target_that_is_edited_during_the_copy_is_kept() {
    let (pair, request) = replacing_old();
    let edited = pair.right().join("doc.bin");

    let ended = rig_acting_once(writes(edited, b"edited meanwhile, newer than the scan")).run(request);

    is_done(&ended, 1);
    assert_eq!(
        read(&pair.right().join("doc.bin")),
        b"edited meanwhile, newer than the scan"
    );
    assert!(has_no_partial_files(pair.root.path()));
}

/// What a user does to `path`: writes `bytes` to it, in place.
fn writes(path: PathBuf, bytes: &'static [u8]) -> impl Fn() + Send + Sync + 'static {
    move || fs::write(&path, bytes).expect("write")
}

/// What a user does to `path`: puts another file there that has the size and time (`seconds`) of what the scan saw.
fn lookalike(path: PathBuf, bytes: &'static [u8], seconds: u64) -> impl Fn() + Send + Sync + 'static {
    move || {
        let aside = path.with_extension("aside");

        fs::write(&aside, bytes).expect("write");
        set_times(&aside, moment(seconds));
        fs::rename(&aside, &path).expect("rename");
    }
}

#[test]
fn a_target_replaced_by_another_file_of_the_same_size_and_time_is_kept() {
    let (pair, request) = replacing_old();
    let swapped = pair.right().join("doc.bin");

    let ended = rig_acting_once(lookalike(swapped, b"new", 200)).run(request);

    is_done(&ended, 1);
    assert_eq!(read(&pair.right().join("doc.bin")), b"new");
    assert_eq!(modified(&pair.right().join("doc.bin")), moment(200));
    assert!(has_no_partial_files(pair.root.path()));
}

#[test]
fn a_target_that_is_still_the_scanned_file_is_replaced() {
    let (pair, request) = replacing_old();

    let ended = rig_acting_once(|| {}).run(request);

    is_done(&ended, 0);
    assert_eq!(read(&pair.right().join("doc.bin")), vec![7; BIG]);
}

#[test]
fn a_file_that_appears_inside_a_folder_made_for_the_copy_is_kept() {
    let pair = Pair::new();
    file(&pair.left().join("tree/a/big.bin"), &vec![7; BIG], 100);
    file(&pair.left().join("tree/b/same.txt"), b"from left", 100);
    let request = sync_of(&pair.left(), &pair.right(), &["tree"], ToRight);
    let mine = pair.right().join("tree/b/same.txt");

    let ended = rig_acting_once(move || {
        fs::create_dir_all(mine.parent().expect("parent")).expect("folder");
        fs::write(&mine, b"mine").expect("write");
    })
    .run(request);

    is_done(&ended, 1);
    assert_eq!(read(&pair.right().join("tree/b/same.txt")), b"mine");
    assert_eq!(read(&pair.right().join("tree/a/big.bin")), vec![7; BIG]);
    assert!(has_no_partial_files(pair.root.path()));
}

#[cfg(unix)]
#[test]
fn a_target_folder_swapped_for_a_link_before_the_write_is_not_followed() {
    let pair = Pair::new();
    let outside = super::testing::temp();
    file(&pair.left().join("deep/inside.txt"), b"inside", 100);
    fs::create_dir_all(pair.right().join("deep")).expect("folder");
    let request = sync_of(&pair.left(), &pair.right(), &["deep/inside.txt"], ToRight);

    let (folder, away) = (pair.right().join("deep"), outside.path().to_owned());
    let hooks = Hooks {
        before_open: Some(Arc::new(move |rel| {
            if rel == "deep" {
                fs::remove_dir(&folder).expect("remove");
                std::os::unix::fs::symlink(&away, &folder).expect("link");
            }
        })),
        ..Hooks::default()
    };
    let ended = Rig::with(hooks, |_, _| {}).run(request);

    assert_eq!(ended.errors.len() + ended.skipped as usize, 1, "{:?}", ended.errors);
    assert!(tree(outside.path()).is_empty());
    assert_eq!(read(&pair.left().join("deep/inside.txt")), b"inside");
}

#[cfg(unix)]
#[test]
fn a_folder_made_for_the_copy_is_entered_by_handle() {
    let pair = Pair::new();
    file(&pair.left().join("a/b/c.txt"), b"c", 100);

    let ended = Rig::new().run(sync_of(&pair.left(), &pair.right(), &["a/b/c.txt"], ToRight));

    is_done(&ended, 0);
    assert_eq!(tree(&pair.right()), ["a/", "a/b/", "a/b/c.txt"]);
}

#[cfg(unix)]
#[test]
fn terms_tell_a_lookalike_apart_and_fail_closed_without_an_identity() {
    use super::sync_terms::{Terms, seen_at};

    let pair = Pair::new();
    file(&pair.left().join("a"), b"same", 100);
    file(&pair.left().join("b"), b"same", 100);
    let (a, b) = (seen_at(&pair.left().join("a")), seen_at(&pair.left().join("b")));
    let (a, b) = (a.expect("a").expect("there"), b.expect("b").expect("there"));
    assert_eq!(a.0, b.0, "the two files have the same facts");

    let terms = Terms::judge(Some(&a.0), Some(a)).expect("as the plan says");
    let unknown = Terms::judge(Some(&a.0), Some((a.0, None))).expect("as the plan says");

    assert!(terms.allows(Some(&a)));
    assert!(!terms.allows(Some(&b)));
    assert!(!terms.allows(Some(&(a.0, None))));
    assert!(!terms.allows(None));
    assert!(!unknown.allows(Some(&a)));
    assert_eq!(Terms::judge(None, None), Some(Terms::Absent));
    assert_eq!(Terms::judge(None, Some(a)), None);
}
