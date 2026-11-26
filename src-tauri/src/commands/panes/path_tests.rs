//! Tests of the check that the folders on a pane's path do not lead to a network or device path.

use super::run::local_folder;
use super::*;

/// A folder in the system's temp folder, removed on drop (the app has no `tempfile` dependency).
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("pairpane-panes-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&path).ok();
        std::fs::create_dir_all(&path).expect("scratch folder");

        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// Only Windows connects to a host by resolving a path, so only there is the path checked; elsewhere every path is
/// left to the listing itself, exactly as before.
#[cfg(not(windows))]
#[test]
fn paths_are_not_checked_beforehand_off_windows() {
    let scratch = Scratch::new("plain");
    std::fs::create_dir(scratch.0.join("sub")).expect("dir");

    assert!(local_folder(&scratch.0.join("sub")).is_ok());
    assert!(local_folder(&scratch.0.join("missing")).is_ok());
}

#[cfg(unix)]
#[test]
fn a_path_through_a_folder_link_lists_and_a_looping_one_fails_to_list() {
    use std::os::unix::fs::symlink;

    let scratch = Scratch::new("links");
    std::fs::create_dir_all(scratch.0.join("real/sub")).expect("dirs");
    symlink(scratch.0.join("real"), scratch.0.join("alias")).expect("link");
    symlink("loop", scratch.0.join("loop")).expect("link");

    assert!(local_folder(&scratch.0.join("alias/sub")).is_ok());
    let list = |folder: &Path| platform::local_path(folder).and_then(|path| std::fs::read_dir(path).map(drop));
    assert!(list(&scratch.0.join("alias/sub")).is_ok());
    assert!(list(&scratch.0.join("loop/sub")).is_err());
}

#[cfg(windows)]
#[test]
fn a_folder_on_an_ordinary_path_can_be_opened_and_a_missing_one_says_so() {
    let scratch = Scratch::new("plain");
    std::fs::create_dir(scratch.0.join("sub")).expect("dir");

    assert!(local_folder(&scratch.0.join("sub")).is_ok());
    let missing = local_folder(&scratch.0.join("missing")).expect_err("missing");
    assert_eq!(missing.code, IpcErrorCode::NotFound);
}

/// Needs a Windows symbolic link, which can need a privilege: skipped when one cannot be made.
#[cfg(windows)]
#[test]
fn a_folder_behind_a_link_to_a_network_share_is_refused() {
    use std::os::windows::fs::symlink_dir;

    let scratch = Scratch::new("network");
    for remote in [r"\\?\UNC\attacker\share", r"\\attacker\share", r"\\.\pipe\attacker"] {
        let link = scratch.0.join("share");
        std::fs::remove_dir(&link).ok();
        if symlink_dir(remote, &link).is_err() {
            return;
        }

        for folder in [link.clone(), link.join("deep").join("er")] {
            let error = local_folder(&folder).expect_err(remote);
            assert_eq!(error.code, IpcErrorCode::InvalidInput, "{remote}");
        }
    }
}

/// The folder is read at the path without links, not at the one with them: Windows itself then follows none.
/// Needs a Windows symbolic link, which can need a privilege: skipped when one cannot be made.
#[cfg(windows)]
#[test]
fn a_folder_behind_a_local_link_is_read_where_the_link_leads() {
    use std::os::windows::fs::symlink_dir;

    let scratch = Scratch::new("local-link");
    std::fs::create_dir_all(scratch.0.join("real/sub")).expect("dirs");
    let link = scratch.0.join("alias");
    if symlink_dir(scratch.0.join("real"), &link).is_err() {
        return;
    }

    let local = local_folder(&link.join("sub")).expect("local link");
    let actual = local.actual();

    assert!(!actual.starts_with(&link), "{actual:?} still goes through the link");
    assert_eq!(
        std::fs::canonicalize(actual).expect("actual"),
        std::fs::canonicalize(scratch.0.join("real/sub")).expect("real")
    );
}
