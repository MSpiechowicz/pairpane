//! What bounds the parent's look at the object stores of a repository (see `objects`): the one budget of entries over every folder
//! of every store, a store that many lines name looked at once, the exit flag and the time limit, and a store that belongs to
//! another user.

use std::fs;
#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
use std::sync::atomic::AtomicBool;

use super::tests::{fresh, store};
#[cfg(unix)]
use super::tests::{prepared, read};
use super::*;
use crate::gitstatus::gate::Halt;
use crate::gitstatus::testing;
#[cfg(unix)]
use crate::gitstatus::{Limits, helper::prepare, helper_request, locate::Found, note::Note, tests::clean};

#[test]
fn more_entries_than_the_cap_is_a_repository_too_big_to_read() {
    let folder = testing::tempdir();
    let objects = store(folder.path());
    fs::write(objects.join("ab/second"), b"x").expect("an object");
    fs::write(objects.join("pack/pack-1.pack"), b"p").expect("a pack");

    // Three entries listed: two loose objects and a pack.
    assert!(object_folder(&objects, &mut fresh()).is_ok());
    assert!(object_folder(&objects, &mut Looking::with_entries(Gate::never(), 3)).is_ok());
    assert_eq!(
        object_folder(&objects, &mut Looking::with_entries(Gate::never(), 2)),
        Err(Refusal::TooBig)
    );
}

#[test]
fn the_entries_of_every_folder_of_every_store_count_in_one_budget() {
    let folder = testing::tempdir();
    let (one, two) = (store(&folder.path().join("one")), store(&folder.path().join("two")));
    let (own, third) = (folder.path().join("own"), store(&folder.path().join("three")));
    fs::create_dir_all(own.join("objects/info")).expect("folders");
    fs::write(
        own.join("objects/info/alternates"),
        format!("{}\n{}\n{}\n", one.display(), two.display(), third.display()),
    )
    .expect("a list");
    for objects in [&one, &two, &third] {
        fs::write(objects.join("pack/pack-1.pack"), b"p").expect("a pack");
        fs::create_dir(objects.join("info/commit-graphs")).expect("a folder");
        fs::write(objects.join("info/commit-graphs/graph-1.graph"), b"g").expect("a file");
        fs::create_dir(objects.join("pack/multi-pack-index.d")).expect("a folder");
        fs::write(objects.join("pack/multi-pack-index.d/midx-1.midx"), b"m").expect("a file");
    }

    // Each store lists five entries (a loose object, a pack, the folder of the multi-pack index and its file, a commit graph).
    let looked_at = |entries| alternates(&own, &mut Looking::with_entries(Gate::never(), entries));

    assert!(looked_at(15).is_ok());
    // One entry short, in the last store: no store has more than it may, and the repository does.
    assert_eq!(looked_at(14), Err(Refusal::TooBig));
}

#[test]
fn a_store_that_many_lines_and_a_chain_name_is_looked_at_once() {
    let folder = testing::tempdir();
    let store = folder.path().join("store/objects");
    fs::create_dir_all(store.join("info")).expect("folders");
    let own = folder.path().join("own");
    fs::create_dir_all(own.join("objects/info")).expect("folders");
    let line = format!("{}\n", store.display());
    // 30 lines name it, and so does the store itself (a loop back to the first); 31 are all that a repository may have.
    fs::write(own.join("objects/info/alternates"), line.repeat(30)).expect("a list");
    fs::write(store.join("info/alternates"), format!("{}\n", store.display())).expect("a list");

    let stores = object_folders(&own, Gate::never()).expect("local");

    assert_eq!(stores, [own.join("objects"), store]);
}

/// A store with `count` files in each of `pack`, `pack/multi-pack-index.d` and `info/commit-graphs`, which `lines` lines of the
/// list of alternates of the repository at `own` name (the auditor's `parent-cost`).
#[cfg(unix)]
fn crowded_store(folder: &Path, own: &Path, count: usize, lines: usize) {
    let store = folder.join("store/objects");
    for name in ["pack", "info/commit-graphs", "pack/multi-pack-index.d"] {
        fs::create_dir_all(store.join(name)).expect("folders");
        for index in 0..count {
            fs::write(store.join(name).join(format!("f{index}")), b"").expect("a file");
        }
    }
    let line = format!("{}\n", store.display());

    fs::create_dir_all(own.join("info")).expect("a folder");
    fs::write(own.join("info/alternates"), line.repeat(lines)).expect("a list");
}

#[cfg(unix)]
#[test]
fn a_store_with_many_files_that_every_line_of_the_list_names_costs_what_it_holds_once() {
    let repo = clean(&[("a.txt", b"a")]);
    let folder = testing::tempdir();
    // 48,000 entries in all: under the cap once, and 1,488,000 if it were looked at for each of the 31 lines.
    crowded_store(folder.path(), &repo.git_dir().join("objects"), 16_000, 31);
    let started = std::time::Instant::now();

    let request = helper_request(repo.root(), Limits::default());

    assert!(
        matches!(request, Ok(Some(_))),
        "{:?}",
        request.map(|request| request.map(|bytes| bytes.len()))
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}

#[cfg(unix)]
#[test]
fn a_repository_over_the_cap_of_entries_is_too_big_at_once_however_many_stores_hold_them() {
    let repo = clean(&[("a.txt", b"a")]);
    let folder = testing::tempdir();
    crowded_store(folder.path(), &repo.git_dir().join("objects"), 22_000, 31);
    let started = std::time::Instant::now();

    // 66,000 entries of one store: over the 65,536 a repository may have.
    assert!(matches!(
        helper_request(repo.root(), Limits::default()),
        Err(crate::gitstatus::GitError::TooBig)
    ));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_look_that_is_stopped_ends_with_the_reason_before_it_lists_anything() {
    let folder = testing::tempdir();
    let objects = store(folder.path());
    let cancel = std::sync::atomic::AtomicBool::new(true);

    assert_eq!(
        object_folder(
            &objects,
            &mut Looking::new(Gate::new(&cancel, std::time::Duration::from_secs(60)))
        ),
        Err(Refusal::Stopped(Halt::Cancelled))
    );
    let never = std::sync::atomic::AtomicBool::new(false);
    assert_eq!(
        object_folder(
            &objects,
            &mut Looking::new(Gate::new(&never, std::time::Duration::ZERO))
        ),
        Err(Refusal::Stopped(Halt::TimedOut))
    );
    assert_eq!(
        object_folders(folder.path(), Gate::new(&cancel, std::time::Duration::from_secs(60))),
        Err(Refusal::Stopped(Halt::Cancelled))
    );
}

#[cfg(unix)]
#[test]
fn an_alternate_store_of_another_user_is_refused_with_the_note_of_the_owner() {
    if rustix::process::geteuid().is_root() {
        eprintln!("skipped: the user is root, who owns what is root's");

        return;
    }
    let repo = clean(&[("a.txt", b"a")]);
    fs::create_dir_all(repo.git_dir().join("objects/info")).expect("a folder");
    // A folder of the system, which is root's and no broad place.
    fs::write(repo.git_dir().join("objects/info/alternates"), "/usr/share\n").expect("a list");

    assert_eq!(prepared(repo.root()), Found::Refused(Note::Owner));
    assert_eq!(helper_request(repo.root(), Limits::default()).expect("asked"), None);
    assert_eq!(read(repo.root()), (false, Some(Note::Owner.text().to_owned())));
}

#[cfg(unix)]
#[test]
fn the_look_at_a_repository_ends_at_the_time_limit_and_at_the_exit_flag() {
    let repo = clean(&[("a.txt", b"a")]);
    let no_time = Limits {
        time: std::time::Duration::ZERO,
        ..Limits::default()
    };

    assert!(matches!(
        prepare(repo.root(), None, no_time, &AtomicBool::new(false)),
        Err(crate::gitstatus::GitError::TimedOut)
    ));
    assert!(matches!(
        prepare(repo.root(), None, Limits::default(), &AtomicBool::new(true)),
        Err(crate::gitstatus::GitError::Cancelled)
    ));
    assert!(matches!(prepared(repo.root()), Found::Repository(_)));
}
