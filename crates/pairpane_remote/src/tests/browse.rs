//! What a listing and its calls return.

use std::io::ErrorKind;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use pairpane_core::vfs::{RemoteFs, RemoteKind};
use pretty_assertions::assert_eq;

use super::{Harness, path};
use crate::test_server::{HOME, Settings};

const PASSWORD: &str = "correct horse";

pub(super) fn connected(settings: Settings) -> (Harness, std::sync::Arc<dyn RemoteFs>) {
    let h = Harness::new(settings);

    h.trust();

    let fs = h.pool.fs(h.site(Some(PASSWORD))).expect("a site");

    (h, fs)
}

fn names(fs: &dyn RemoteFs, dir: &str) -> Vec<String> {
    let mut names: Vec<String> = fs
        .read_dir(&path(dir), &AtomicBool::new(false))
        .expect("listed")
        .into_iter()
        .map(|entry| entry.name)
        .collect();

    names.sort();

    names
}

#[test]
fn a_listing_has_kinds_sizes_times_and_modes() {
    let (h, fs) = connected(Settings::default());

    h.server.put("data/notes.txt", b"twelve bytes");
    h.server.put("data/sub/inner.txt", b"x");

    let mut entries = fs.read_dir(&path("/data"), &AtomicBool::new(false)).expect("listed");

    entries.sort_by(|a, b| a.name.cmp(&b.name));

    assert_eq!(
        entries.iter().map(|entry| entry.name.as_str()).collect::<Vec<_>>(),
        ["notes.txt", "sub"]
    );

    let file = &entries[0].meta;
    let folder = &entries[1].meta;

    assert_eq!((file.kind, file.size), (RemoteKind::File, 12));
    assert_eq!((folder.kind, folder.size), (RemoteKind::Dir, 0));

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as i64;

    assert!((now - file.modified_ms.expect("a time")).abs() < 120_000);
    assert!(file.mode.expect("a mode") & 0o400 != 0 && file.mode.expect("a mode") <= 0o7777);
}

#[test]
fn every_page_of_a_long_folder_is_read() {
    let (h, fs) = connected(Settings {
        page_size: 2,
        ..Settings::default()
    });

    for number in 0..7 {
        h.server.put(&format!("many/file{number}"), b"");
    }

    assert_eq!(names(&*fs, "/many").len(), 7);
    assert_eq!(h.server.counts.pages.load(Ordering::Relaxed), 4);
}

#[cfg(unix)]
#[test]
fn a_link_is_a_link_until_it_is_looked_at() {
    let (h, fs) = connected(Settings::default());

    h.server.put("links/target.txt", b"hello");
    h.server.put("links/dir/inner", b"");
    std::os::unix::fs::symlink("target.txt", h.server.root.path().join("links/to-file")).expect("link");
    std::os::unix::fs::symlink("dir", h.server.root.path().join("links/to-dir")).expect("link");
    std::os::unix::fs::symlink("missing", h.server.root.path().join("links/broken")).expect("link");

    let listed = fs.read_dir(&path("/links"), &AtomicBool::new(false)).expect("listed");
    let kind = |name: &str| {
        listed
            .iter()
            .find(|entry| entry.name == name)
            .expect("an entry")
            .meta
            .kind
    };

    assert_eq!(kind("to-file"), RemoteKind::Link);
    assert_eq!(kind("broken"), RemoteKind::Link);
    assert_eq!(kind("target.txt"), RemoteKind::File);

    assert_eq!(
        fs.stat(&path("/links/to-dir")).expect("stat").expect("exists").kind,
        RemoteKind::Dir
    );
    assert_eq!(fs.stat(&path("/links/to-file")).expect("stat").expect("exists").size, 5);
    assert_eq!(fs.stat(&path("/links/broken")).expect("stat"), None);
    assert_eq!(fs.read_link(&path("/links/to-file")).expect("read"), "target.txt");
}

#[test]
fn errors_say_what_happened_without_naming_the_path() {
    let (h, fs) = connected(Settings::default());

    h.server.put("secret-place/locked/x", b"");

    for (folder, kind) in [
        ("/nowhere", ErrorKind::NotFound),
        ("/secret-place/locked", ErrorKind::PermissionDenied),
    ] {
        let error = fs.read_dir(&path(folder), &AtomicBool::new(false)).expect_err("failed");

        assert_eq!(error.kind(), kind);
        assert!(
            !error.to_string().contains("nowhere") && !error.to_string().contains("secret-place"),
            "{error}"
        );
    }

    assert_eq!(fs.stat(&path("/nowhere")).expect("stat"), None);
    assert!(fs.read_link(&path("/nowhere")).is_err());
}

#[test]
fn the_home_folder_is_where_listing_can_start() {
    let (h, fs) = connected(Settings::default());

    h.server.put(&format!("{HOME}/.profile"), b"");

    assert_eq!(names(&*fs, HOME), [".profile"]);
}

#[test]
fn a_cancelled_listing_stops_between_pages() {
    let (h, fs) = connected(Settings {
        page_size: 1,
        page_delay: Duration::from_millis(60),
        ..Settings::default()
    });

    for number in 0..30 {
        h.server.put(&format!("slow/file{number}"), b"");
    }

    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let lister = {
        let (fs, cancel) = (fs.clone(), cancel.clone());

        std::thread::spawn(move || fs.read_dir(&path("/slow"), &cancel))
    };

    std::thread::sleep(Duration::from_millis(250));
    cancel.store(true, Ordering::Relaxed);

    let error = lister.join().expect("the thread").expect_err("interrupted");

    assert_eq!(error.kind(), ErrorKind::Interrupted);
    assert!(h.server.counts.pages.load(Ordering::Relaxed) < 30);
}

// Only where the file system takes any bytes in a name: APFS refuses one that is not valid text (`names.rs` has the unit
// test of the filter itself).
#[cfg(target_os = "linux")]
#[test]
fn a_name_that_is_not_valid_text_is_left_out_of_the_listing() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let (h, fs) = connected(Settings::default());

    h.server.put("mixed/fine.txt", b"");
    h.server.put("mixed/also fine", b"");
    std::fs::write(
        h.server
            .root
            .path()
            .join("mixed")
            .join(OsStr::from_bytes(b"bad\xff\xfename")),
        b"",
    )
    .expect("a file");

    assert_eq!(names(&*fs, "/mixed"), ["also fine", "fine.txt"]);
}
