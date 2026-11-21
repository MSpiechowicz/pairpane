use std::fs;
use std::os::fd::OwnedFd;
use std::path::Path;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::super::handle::{Facts, open_folder, open_parent};
use super::top_of;

fn folder(path: &Path) -> OwnedFd {
    open_folder(path).expect("folder")
}

fn facts(path: &Path) -> Facts {
    Facts::of_handle(&folder(path)).expect("facts")
}

#[test]
fn the_top_of_the_home_is_the_top_of_the_data_folder_the_home_trash_is_in() {
    let Some(home) = std::env::home_dir() else { return };
    let share = home.join(".local/share");
    // A data folder that is a drive of its own has a top of its own.
    if !share.is_dir() || facts(&share).device != facts(&home).device {
        return;
    }

    let home_top = top_of(&folder(&home)).expect("top of the home");
    let share_top = top_of(&folder(&share)).expect("top of the data folder");

    assert!(
        Facts::of_handle(&home_top.fd)
            .expect("facts")
            .is_same_place(&Facts::of_handle(&share_top.fd).expect("facts"))
    );
}

#[test]
fn the_top_is_on_the_drive_of_the_folder_and_its_parent_is_not_or_it_is_the_root() {
    let temp = TempDir::new().expect("temp dir");
    let deep = temp.path().join("a/b/c");
    fs::create_dir_all(&deep).expect("folders");

    for start in [deep.as_path(), temp.path(), Path::new("/"), Path::new("/proc")] {
        let handle = folder(start);

        let top = top_of(&handle).expect("top");

        let here = Facts::of_handle(&handle).expect("facts");
        let at = Facts::of_handle(&top.fd).expect("facts");
        assert_eq!(at.device, here.device, "{start:?}");
        let above = Facts::of_handle(&open_parent(&top.fd).expect("parent")).expect("facts");
        assert!(above.device != at.device || above.is_same_place(&at), "{start:?}");
    }
}

#[test]
fn the_steps_up_lead_to_the_top_by_its_path() {
    let temp = TempDir::new().expect("temp dir");
    let deep = fs::canonicalize(temp.path()).expect("canonical").join("a/b/c");
    fs::create_dir_all(&deep).expect("folders");

    let top = top_of(&folder(&deep)).expect("top");

    let by_path = deep.ancestors().nth(top.steps).expect("an ancestor");
    assert!(facts(by_path).is_same_place(&Facts::of_handle(&top.fd).expect("facts")));
}

#[test]
fn the_root_is_a_top_at_once() {
    let top = top_of(&folder(Path::new("/"))).expect("top");

    assert_eq!(top.steps, 0);
}
