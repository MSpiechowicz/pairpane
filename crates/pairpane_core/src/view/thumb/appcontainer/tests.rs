//! The grant that lets the container run the helper's file, tried for real: a copy of the test program in a folder whose
//! access list has the current user and the system and nothing else (not ALL APPLICATION PACKAGES, which a build folder may
//! have by inheritance and which would let the first start work with no grant).

use std::io::Read;
use std::path::Path;
use std::process::Command;

use crate::process::{Started, spawn};
use crate::view::{is_refusal, is_transient};

use super::MEMORY_FOR_TESTS as MEMORY;
use super::container::{allow_read_and_execute, container_sid, entries_of};
use super::start;

/// A folder that only the current user and the system may use: its access list is its own (inheritance is removed) and has
/// those two, with full control that its files inherit. Done by `icacls`, a program of the system started with fixed
/// arguments and no shell.
fn restricted_folder() -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("a folder");
    let root = std::env::var_os("SystemRoot").expect("the system folder");
    let user = std::env::var("USERNAME").expect("the user");

    let mut icacls = Command::new(Path::new(&root).join("System32").join("icacls.exe"));
    icacls
        .arg(folder.path())
        .args(["/inheritance:r", "/grant:r", "*S-1-5-18:(OI)(CI)F", "/grant:r"])
        .arg(format!("{user}:(OI)(CI)F"));
    let status = spawn(&mut icacls).expect("icacls starts").wait().expect("icacls ends");
    assert!(status.success(), "the folder is restricted: {status:?}");

    folder
}

/// A copy of the test program in a restricted folder, which the container has no entry for.
fn copy_in(folder: &Path) -> std::path::PathBuf {
    let copy = folder.join("helper.exe");
    std::fs::copy(std::env::current_exe().expect("the test program"), &copy).expect("a copy");

    copy
}

#[test]
fn the_grant_adds_one_entry_and_a_second_grant_adds_none() {
    let folder = restricted_folder();
    let copy = copy_in(folder.path());
    let container = container_sid().expect("the container");
    assert_eq!(
        entries_of(&copy, &container).expect("the list"),
        0,
        "no entry of the container yet"
    );

    allow_read_and_execute(&copy, &container).expect("the grant");
    assert_eq!(entries_of(&copy, &container).expect("the list"), 1, "one entry");

    allow_read_and_execute(&copy, &container).expect("the grant again");
    assert_eq!(
        entries_of(&copy, &container).expect("the list"),
        1,
        "still one: nothing was written the second time"
    );
}

/// The start of a file the container has no entry for. Where the system refuses it (access denied) the grant is made and
/// the start works, with one entry; where the system does not (it allows the container by some other entry of the list, or
/// traverses the folders on the way), the start works with none. Where the container cannot run the file even with the
/// grant (a folder higher up refuses it) the start is refused as unavailable and nothing is started: that is the fail
/// closed outcome, and it is the only other one there is. In every case a second grant writes nothing.
#[test]
fn a_file_the_container_is_denied_is_granted_once_and_started_or_refused_and_never_started_outside() {
    let folder = restricted_folder();
    let copy = copy_in(folder.path());
    let container = container_sid().expect("the container");

    match start(&copy, &["--list"], MEMORY) {
        Ok((mut helper, stdin, mut stdout)) => {
            drop(stdin);
            let mut text = String::new();
            stdout.read_to_string(&mut text).expect("the list of tests is read");
            let status = helper.wait().expect("the helper ends");

            assert!(status.success(), "{status:?} {text}");
            assert!(text.contains("tests"), "it ran: {text}");
        }
        Err(error) => {
            assert_eq!(error.kind(), std::io::ErrorKind::Unsupported, "{error:?}");
            assert!(
                is_refusal(&error) && is_transient(&error),
                "refused, not remembered: {error:?}"
            );
        }
    }

    let after = entries_of(&copy, &container).expect("the list");
    eprintln!(
        "entries of the container after the first start: {after} (1: it was denied and granted, 0: it was not denied)"
    );
    assert!(after <= 1, "never more than one entry");

    allow_read_and_execute(&copy, &container).expect("the grant again");
    assert_eq!(
        entries_of(&copy, &container).expect("the list"),
        1,
        "the grant leaves exactly one entry, and a grant that was made before is not made twice"
    );
}
