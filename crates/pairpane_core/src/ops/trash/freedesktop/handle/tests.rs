use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::fs::symlink;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::super::testing::{is_root, me, set_mode};
use super::{
    Facts, Rule, Step, birth_of, exists_in, open_folder, open_folder_exact, open_parent, rename_noreplace, step,
    step_or_make,
};

fn facts(is_folder: bool, owner: u32, mode: u32) -> Facts {
    Facts {
        is_folder,
        owner,
        mode,
        device: 1,
        inode: 1,
    }
}

fn folder(path: &std::path::Path) -> OwnedFd {
    open_folder(path).expect("folder")
}

#[test]
fn a_folder_is_guarded_when_nobody_else_can_rename_what_is_in_it() {
    // The owner is root or the user; the mode keeps the group and others out, or the sticky bit keeps them to their own.
    let table = [
        // A private folder of the user, and a folder of root.
        (true, 1000, 0o700, true),
        (true, 0, 0o755, true),
        // The modes of a FAT or exFAT drive that is mounted by the user: nobody else can write.
        (true, 1000, 0o755, true),
        (true, 1000, 0o555, true),
        (true, 1000, 0o750, true),
        // Open to others and not sticky.
        (true, 1000, 0o777, false),
        (true, 1000, 0o775, false),
        (true, 1000, 0o757, false),
        (true, 0, 0o772, false),
        // Sticky keeps others to their own entries.
        (true, 0, 0o1777, true),
        (true, 1000, 0o1777, true),
        (true, 1000, 0o1775, true),
        // Someone else's.
        (true, 1001, 0o700, false),
        (true, 1001, 0o1777, false),
        // Not a folder (a link or a file).
        (false, 1000, 0o700, false),
    ];

    for (is_folder, owner, mode, guarded) in table {
        assert_eq!(
            facts(is_folder, owner, mode).is_guarded(1000),
            guarded,
            "{owner} {mode:o} folder: {is_folder}"
        );
    }
}

#[test]
fn the_rules_ask_for_more_than_a_guarded_folder_where_they_must() {
    // `Own`: the user's own, not root's. `Shared`: sticky as well.
    assert!(Rule::Guarded.allows(&facts(true, 0, 0o755), 1000));
    assert!(!Rule::Own.allows(&facts(true, 0, 0o755), 1000));
    assert!(Rule::Own.allows(&facts(true, 1000, 0o755), 1000));
    assert!(!Rule::Own.allows(&facts(true, 1000, 0o777), 1000));
    assert!(Rule::Shared.allows(&facts(true, 0, 0o1777), 1000));
    assert!(Rule::Shared.allows(&facts(true, 1000, 0o1755), 1000));
    assert!(!Rule::Shared.allows(&facts(true, 0, 0o755), 1000));
}

#[test]
fn a_step_opens_a_real_folder_and_reports_what_is_missing_or_not_to_be_used() {
    let temp = TempDir::new().expect("temp dir");
    let top = folder(temp.path());
    fs::create_dir(temp.path().join("real")).expect("folder");
    fs::write(temp.path().join("file"), "").expect("file");
    symlink(temp.path().join("real"), temp.path().join("link")).expect("link");
    symlink(temp.path().join("nowhere"), temp.path().join("dangling")).expect("link");

    assert!(matches!(step(&top, OsStr::new("real"), Rule::Own, me()), Step::Open(_)));
    assert!(matches!(
        step(&top, OsStr::new("absent"), Rule::Own, me()),
        Step::Missing
    ));
    for name in ["file", "link", "dangling"] {
        assert!(
            matches!(step(&top, OsStr::new(name), Rule::Own, me()), Step::Refused),
            "{name}"
        );
    }
}

#[test]
fn a_step_judges_the_folder_it_opened_by_its_owner_and_mode() {
    let temp = TempDir::new().expect("temp dir");
    let top = folder(temp.path());
    fs::create_dir(temp.path().join("mine")).expect("folder");

    // The folder is the tester's: to another user it is a stranger's.
    assert!(matches!(
        step(&top, OsStr::new("mine"), Rule::Own, me() + 1),
        Step::Refused
    ));

    set_mode(&temp.path().join("mine"), 0o777);
    assert!(matches!(step(&top, OsStr::new("mine"), Rule::Own, me()), Step::Refused));
    set_mode(&temp.path().join("mine"), 0o1777);
    assert!(matches!(step(&top, OsStr::new("mine"), Rule::Own, me()), Step::Open(_)));
}

#[test]
fn a_missing_folder_is_made_private_and_opened() {
    use std::os::unix::fs::MetadataExt;

    let temp = TempDir::new().expect("temp dir");
    let top = folder(temp.path());

    let made = step_or_make(&top, OsStr::new("new"), Rule::Own, me());

    assert!(matches!(made, Step::Open(_)));
    assert_eq!(
        fs::metadata(temp.path().join("new")).expect("made").mode() & 0o777,
        0o700
    );
    // What is there is left as it is.
    assert!(matches!(
        step_or_make(&top, OsStr::new("new"), Rule::Own, me()),
        Step::Open(_)
    ));
}

#[test]
fn a_folder_that_cannot_be_made_is_refused() {
    if is_root() {
        return;
    }
    let temp = TempDir::new().expect("temp dir");
    let top = folder(temp.path());
    set_mode(temp.path(), 0o555);

    let made = step_or_make(&top, OsStr::new("new"), Rule::Own, me());

    set_mode(temp.path(), 0o755);
    assert!(matches!(made, Step::Refused));
}

#[test]
fn a_folder_is_opened_from_its_handle_even_when_its_path_is_taken_over() {
    let temp = TempDir::new().expect("temp dir");
    fs::create_dir(temp.path().join("kept")).expect("folder");
    let held = folder(&temp.path().join("kept"));
    let before = Facts::of_handle(&held).expect("facts");

    // The name is moved away and another folder takes it.
    fs::rename(temp.path().join("kept"), temp.path().join("aside")).expect("aside");
    fs::create_dir(temp.path().join("kept")).expect("again");

    assert_eq!(Facts::of_handle(&held).expect("facts"), before);
    assert!(!folder(&temp.path().join("kept")).eq_place(&held));
    assert!(open_parent(&held).is_ok());
}

/// A handle compared by what it is, not by its name.
trait SamePlace {
    fn eq_place(&self, other: &OwnedFd) -> bool;
}

impl SamePlace for OwnedFd {
    fn eq_place(&self, other: &OwnedFd) -> bool {
        Facts::of_handle(self)
            .expect("facts")
            .is_same_place(&Facts::of_handle(other).expect("facts"))
    }
}

#[test]
fn a_rename_replaces_nothing() {
    let temp = TempDir::new().expect("temp dir");
    let dir = folder(temp.path());
    fs::write(temp.path().join("from"), "from").expect("file");
    fs::write(temp.path().join("taken"), "taken").expect("file");

    let refused = rename_noreplace(&dir, OsStr::new("from"), &dir, OsStr::new("taken"));

    assert_eq!(refused.expect_err("taken").kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(temp.path().join("taken")).expect("kept"), "taken");

    rename_noreplace(&dir, OsStr::new("from"), &dir, OsStr::new("free")).expect("renamed");
    assert_eq!(fs::read_to_string(temp.path().join("free")).expect("moved"), "from");
    assert!(!exists_in(&dir, OsStr::new("from")).expect("looked"));
}

#[test]
fn a_link_that_leads_nowhere_counts_as_there() {
    let temp = TempDir::new().expect("temp dir");
    let dir = folder(temp.path());
    symlink(temp.path().join("nowhere"), temp.path().join("link")).expect("link");
    fs::write(temp.path().join("from"), "from").expect("file");

    assert!(exists_in(&dir, OsStr::new("link")).expect("looked"));
    let refused = rename_noreplace(&dir, OsStr::new("from"), &dir, OsStr::new("link"));

    assert_eq!(refused.expect_err("taken").kind(), io::ErrorKind::AlreadyExists);
}

#[test]
fn a_folder_opened_exactly_is_never_a_link() {
    let temp = TempDir::new().expect("temp dir");
    fs::create_dir(temp.path().join("real")).expect("folder");
    symlink(temp.path().join("real"), temp.path().join("link")).expect("link");

    assert!(open_folder_exact(&temp.path().join("real")).is_ok());
    assert!(open_folder_exact(&temp.path().join("link")).is_err());
    // The path of the user may lead through links, and is followed.
    assert!(open_folder(&temp.path().join("link")).is_ok());
}

#[test]
fn the_birth_time_of_a_folder_is_told_only_when_the_system_does_and_does_not_change() {
    let temp = TempDir::new().expect("temp dir");
    let fd = folder(temp.path());

    let first = birth_of(&fd);

    assert_eq!(birth_of(&fd), first);
    if let Some((seconds, nanoseconds)) = first {
        assert!(seconds > 0 && nanoseconds < 1_000_000_000, "{seconds} {nanoseconds}");
    }
}
