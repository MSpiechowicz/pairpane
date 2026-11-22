//! The staging folder of an unpacking, and the sweep of the ones a dead run left.

use std::fs;
use std::path::Path;
#[cfg(unix)]
use std::time::{Duration, SystemTime};

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::stage::Staging;
#[cfg(unix)]
use super::stage::{sweep_in, touch};

fn names(folder: &Path) -> Vec<String> {
    let mut names = Vec::new();

    for entry in fs::read_dir(folder).expect("folder").flatten() {
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();

    names
}

/// A folder named like a staging folder of the process `pid`, with a file in it.
#[cfg(unix)]
fn leftover(temp: &Path, pid: u32, nonce: &str) -> String {
    let name = format!(".pairpane-rar-{pid:x}-{nonce}");
    fs::create_dir(temp.join(&name)).expect("folder");
    fs::write(temp.join(&name).join("big.bin"), "x").expect("file");

    name
}

/// A pid that no process has.
#[cfg(target_os = "linux")]
const DEAD: u32 = 0x7fff_fff0;

#[test]
fn the_staging_folder_is_removed_with_its_guard() {
    let temp = TempDir::new().expect("temp");
    let staging = Staging::create(temp.path()).expect("created");
    fs::write(staging.path().join("f"), "x").expect("file");
    assert_eq!(names(temp.path()).len(), 1);

    drop(staging);

    assert_eq!(names(temp.path()), Vec::<String>::new());
}

#[cfg(target_os = "linux")]
#[test]
fn the_sweep_takes_the_folders_of_dead_processes_and_leaves_the_rest() {
    let temp = TempDir::new().expect("temp");
    let dead = leftover(temp.path(), DEAD, "1");
    let alive = leftover(temp.path(), std::process::id(), "2");
    fs::write(temp.path().join(".pairpane-rar-plain-file"), "x").expect("file");
    fs::create_dir(temp.path().join("other")).expect("folder");
    let guard = std::os::unix::fs::symlink(temp.path().join("other"), temp.path().join(".pairpane-rar-ff-link"));
    guard.expect("link");

    sweep_in(temp.path(), SystemTime::now(), false);

    let left = names(temp.path());
    assert!(!left.contains(&dead), "{left:?}");
    assert!(left.contains(&alive), "a running process keeps its folder: {left:?}");
    assert!(left.contains(&".pairpane-rar-plain-file".to_owned()), "{left:?}");
    assert!(
        left.contains(&".pairpane-rar-ff-link".to_owned()),
        "a link is never followed: {left:?}"
    );
    assert!(temp.path().join("other").is_dir());
}

#[cfg(unix)]
#[test]
fn the_sweep_on_exit_takes_this_process_s_folders() {
    let temp = TempDir::new().expect("temp");
    let mine = leftover(temp.path(), std::process::id(), "1");
    let other = leftover(temp.path(), 1, "2");

    sweep_in(temp.path(), SystemTime::now(), false);
    assert!(names(temp.path()).contains(&mine));
    assert!(names(temp.path()).contains(&other));

    sweep_in(temp.path(), SystemTime::now(), true);
    assert!(!names(temp.path()).contains(&mine));
    assert!(
        names(temp.path()).contains(&other),
        "the folder of another process is not ours to take"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_running_process_keeps_its_folder_however_old_it_is() {
    let temp = TempDir::new().expect("temp");
    let live = leftover(temp.path(), 1, "1");
    let dead = leftover(temp.path(), DEAD, "2");

    // Seen from eight hours later every folder is old; only the one of a process that is gone is taken.
    sweep_in(temp.path(), SystemTime::now() + Duration::from_secs(8 * 60 * 60), false);

    assert_eq!(names(temp.path()), [live]);
    assert!(!names(temp.path()).contains(&dead));
}

#[cfg(all(unix, not(target_os = "linux")))]
#[test]
fn where_the_system_does_not_tell_an_old_folder_goes_and_a_young_one_stays() {
    let temp = TempDir::new().expect("temp");
    let other = leftover(temp.path(), 1, "1");

    sweep_in(temp.path(), SystemTime::now(), false);
    assert!(names(temp.path()).contains(&other));

    sweep_in(temp.path(), SystemTime::now() + Duration::from_secs(8 * 60 * 60), false);
    assert_eq!(names(temp.path()), Vec::<String>::new());
}

#[cfg(unix)]
#[test]
fn touching_a_folder_makes_it_young_and_leaves_nothing_in_it() {
    let temp = TempDir::new().expect("temp");
    let staging = Staging::create(temp.path()).expect("created");
    let old = SystemTime::now() - Duration::from_secs(8 * 60 * 60);
    fs::File::open(staging.path())
        .expect("folder")
        .set_modified(old)
        .expect("old");
    let before = fs::metadata(staging.path()).expect("seen").modified().expect("time");
    assert!(before <= old + Duration::from_secs(1));

    touch(staging.path());

    let after = fs::metadata(staging.path()).expect("seen").modified().expect("time");
    assert!(SystemTime::now().duration_since(after).expect("past") < Duration::from_secs(60));
    assert_eq!(names(staging.path()), Vec::<String>::new());
}
