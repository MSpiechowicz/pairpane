//! What `thumb_get` checks before a thread is taken, the caps, the cache in front of the helper and what the page is
//! told. The helper itself, as a real process, is tried by `tests/thumb_helper.rs` and the core crate's tests.

use std::io;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::SystemTime;

use pairpane_core::model::IpcErrorCode;
use pairpane_core::view::thumb;
use pairpane_core::view::{MAX_IMAGE_BYTES, refusal};
use tauri::async_runtime::block_on;

use super::*;
use crate::commands::limit::Permit;
use crate::commands::testing::folder_with;

/// A PNG header and a few bytes: a picture by its first bytes, which no decoder would take.
const PNG_HEAD: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x10\0\0\0\x10\x08\x06\0\0\0";

/// A HEIC file by its brands, which no decoder would take.
const HEIC_HEAD: &[u8] = b"\0\0\0\x18ftypheic\0\0\0\0mif1heic\0\0\0\0";

/// Where a helper would be; what must not start one never reaches it.
fn nowhere() -> PathBuf {
    PathBuf::from("/nonexistent/pairpane-helper")
}

fn fill(cap: &ThreadCap, max: usize) -> Vec<Permit> {
    (0..max).map(|_| cap.acquire(max).expect("a place")).collect()
}

fn get(thumbs: &Thumbs, path: &str, name: &str, size: u32) -> CommandResult<Arc<[u8]>> {
    block_on(thumbs.get(path.to_owned(), name.to_owned(), size, nowhere()))
}

fn shown(result: CommandResult<Arc<[u8]>>) -> (IpcErrorCode, String) {
    let error = result.expect_err("refused");
    let json = serde_json::to_value(&error).expect("json");

    (error.code(), json["message"].as_str().expect("message").to_owned())
}

/// What is kept for the file `name` of `folder` as a picture of `size`, as the command would have kept it.
fn keep(thumbs: &Thumbs, folder: &std::path::Path, name: &str, size: u32, value: Cached) {
    let source = thumb::open(folder, name).expect("a picture");
    let key = Key {
        path: folder.join(name),
        size,
    };
    let stamp = Stamp {
        len: source.len,
        modified: source.modified,
        identity: source.identity,
    };

    thumbs.cache().insert(key, stamp, value);
}

fn picture(bytes: &[u8]) -> Cached {
    Cached::Picture(bytes.into())
}

impl Thumbs {
    fn cache(&self) -> MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[test]
fn what_the_page_sent_is_checked_before_a_thread_is_taken() {
    let (folder, path) = folder_with(&[("a.png", PNG_HEAD)]);
    let thumbs = Thumbs::default();
    let _full = fill(&thumbs.lookups, LOOKUP_THREADS);

    for name in ["../a.png", "sub/a.png", "..", "", "a/../a.png"] {
        let (code, _) = shown(get(&thumbs, &path, name, 128));
        assert_eq!(code, IpcErrorCode::InvalidInput, "{name:?}");
    }
    let (code, _) = shown(get(&thumbs, "relative/folder", "a.png", 128));
    assert_eq!(code, IpcErrorCode::InvalidInput, "a folder that is not absolute");
    let (code, _) = shown(get(&thumbs, &path, &"x".repeat(5000), 128));
    assert_eq!(code, IpcErrorCode::InvalidInput, "a name that is too long");

    // A request that passes the checks is stopped by the full cap, which is a different answer.
    let (code, _) = shown(get(&thumbs, &path, "a.png", 128));
    assert_eq!(code, IpcErrorCode::Busy);
    drop(folder);
}

#[test]
fn a_file_that_is_not_a_picture_is_refused_without_a_helper_even_when_the_helpers_are_all_busy() {
    let (_folder, path) = folder_with(&[
        ("notes.txt", b"hello"),
        ("fake.png", b"this is not a png"),
        ("movie.mp4", b"\0\0\0\x18ftypmp42\0\0\0\0mp42isom"),
    ]);
    let thumbs = Thumbs::default();
    let _helpers = fill(&thumbs.helpers, HELPER_THREADS);

    for name in ["notes.txt", "fake.png", "movie.mp4"] {
        let (code, message) = shown(get(&thumbs, &path, name, 128));

        assert_eq!(code, IpcErrorCode::Unsupported, "{name}");
        assert_eq!(message, "the file is not a picture that has a thumbnail");
    }
}

#[test]
fn a_folder_a_missing_file_and_a_link_are_refused_with_no_path_in_the_text() {
    let (folder, path) = folder_with(&[("a.png", PNG_HEAD)]);
    std::fs::create_dir(folder.path().join("dir.png")).expect("a folder");
    #[cfg(unix)]
    std::os::unix::fs::symlink("a.png", folder.path().join("link.png")).expect("a link");
    let thumbs = Thumbs::default();

    let mut names = vec!["dir.png", "missing.png"];
    if cfg!(unix) {
        names.push("link.png");
    }
    for name in names {
        let (code, message) = shown(get(&thumbs, &path, name, 128));

        assert!(
            matches!(code, IpcErrorCode::InvalidInput | IpcErrorCode::NotFound),
            "{name}: {code:?}"
        );
        assert!(!message.contains(&path), "{name}: {message}");
    }
}

#[test]
fn a_full_lookup_cap_refuses_as_busy() {
    let (_folder, path) = folder_with(&[("a.png", PNG_HEAD)]);
    let thumbs = Thumbs::default();
    let _full = fill(&thumbs.lookups, LOOKUP_THREADS);

    assert_eq!(shown(get(&thumbs, &path, "a.png", 128)).0, IpcErrorCode::Busy);
}

#[test]
fn a_miss_with_every_helper_busy_is_busy_and_a_hit_is_not_held_up_by_them() {
    let (folder, path) = folder_with(&[("a.png", PNG_HEAD), ("b.png", PNG_HEAD)]);
    let thumbs = Thumbs::default();
    keep(&thumbs, folder.path(), "a.png", 128, picture(b"the picture of a"));
    let _helpers = fill(&thumbs.helpers, HELPER_THREADS);

    let (code, message) = shown(get(&thumbs, &path, "b.png", 128));
    assert_eq!(code, IpcErrorCode::Busy);
    assert!(!message.contains(&path));

    let hit = get(&thumbs, &path, "a.png", 128).expect("from the cache");
    assert_eq!(&hit[..], b"the picture of a");
}

#[test]
fn a_heic_picture_has_a_place_of_its_own_among_the_helpers() {
    let (_folder, path) = folder_with(&[("a.png", PNG_HEAD), ("b.heic", HEIC_HEAD)]);
    let thumbs = Thumbs::default();
    let _decoder = fill(&thumbs.heic, HEIC_THREADS);

    assert_eq!(shown(get(&thumbs, &path, "b.heic", 128)).0, IpcErrorCode::Busy);

    // The same cap does not hold up a PNG: it gets as far as starting the helper, which is not there.
    let (code, message) = shown(get(&thumbs, &path, "a.png", 128));
    assert_eq!(code, IpcErrorCode::Io);
    assert_eq!(message, "the file cannot be read");
}

#[test]
fn the_places_are_given_back_when_a_request_ends() {
    let (_folder, path) = folder_with(&[("a.png", PNG_HEAD)]);
    let thumbs = Thumbs::default();

    // Far more requests than there are places, one after another: none leaks a place.
    for _ in 0..(LOOKUP_THREADS + HELPER_THREADS + 5) {
        assert_eq!(shown(get(&thumbs, &path, "a.png", 128)).0, IpcErrorCode::Io);
    }
}

#[test]
fn sizes_that_share_a_bucket_share_a_picture() {
    let (folder, path) = folder_with(&[("a.png", PNG_HEAD)]);
    let thumbs = Thumbs::default();
    keep(&thumbs, folder.path(), "a.png", 128, picture(b"the picture at 128"));

    for size in [97, 100, 110, 128] {
        let picture = get(&thumbs, &path, "a.png", size).expect("a hit");
        assert_eq!(&picture[..], b"the picture at 128", "{size}");
    }

    // Another bucket is another picture, and the helper is not there to make it.
    assert_eq!(shown(get(&thumbs, &path, "a.png", 200)).0, IpcErrorCode::Io);
    // Whatever the page asks, the size is brought into 32..=512.
    for size in [0, 1, u32::MAX] {
        assert_eq!(shown(get(&thumbs, &path, "a.png", size)).0, IpcErrorCode::Io, "{size}");
    }
}

#[test]
fn a_file_that_changed_is_made_again_and_not_served_from_the_cache() {
    let (folder, path) = folder_with(&[("a.png", PNG_HEAD)]);
    let thumbs = Thumbs::default();
    keep(&thumbs, folder.path(), "a.png", 128, picture(b"the old picture"));
    assert!(get(&thumbs, &path, "a.png", 128).is_ok());

    // Longer: the length differs.
    let mut longer = PNG_HEAD.to_vec();
    longer.extend_from_slice(b"more");
    std::fs::write(folder.path().join("a.png"), &longer).expect("rewritten");

    assert_eq!(
        shown(get(&thumbs, &path, "a.png", 128)).0,
        IpcErrorCode::Io,
        "made again"
    );
    assert_eq!(thumbs.cache().bytes(), 0, "and the old picture is gone");
}

#[test]
fn a_file_with_a_new_time_and_the_same_length_is_made_again() {
    let (folder, path) = folder_with(&[("a.png", PNG_HEAD)]);
    let thumbs = Thumbs::default();
    keep(&thumbs, folder.path(), "a.png", 128, picture(b"the old picture"));

    let file = std::fs::File::options()
        .write(true)
        .open(folder.path().join("a.png"))
        .expect("open");
    file.set_modified(SystemTime::now() + std::time::Duration::from_secs(3600))
        .expect("a new time");

    assert_eq!(shown(get(&thumbs, &path, "a.png", 128)).0, IpcErrorCode::Io);
}

#[test]
fn shutting_down_stops_what_runs_and_what_comes_after() {
    let (_folder, path) = folder_with(&[("a.png", PNG_HEAD)]);
    let thumbs = Thumbs::default();

    thumbs.shut_down();

    assert!(thumbs.cancel.load(Ordering::Relaxed));
    let (code, message) = shown(get(&thumbs, &path, "a.png", 128));
    assert_eq!(code, IpcErrorCode::Io);
    assert_eq!(message, "the read was cancelled", "the helper was not started");
}

#[test]
fn what_the_page_is_told_is_fixed_text_that_never_holds_a_path() {
    let secret = "/home/someone/private/photo.png";

    let cases = [
        (
            refusal(
                io::ErrorKind::Unsupported,
                "the file is not a picture that has a thumbnail",
            ),
            IpcErrorCode::Unsupported,
        ),
        (
            refusal(io::ErrorKind::Unsupported, "no heic decoder on this system"),
            IpcErrorCode::Unsupported,
        ),
        (
            refusal(io::ErrorKind::InvalidInput, "the image is too big to show here"),
            IpcErrorCode::InvalidInput,
        ),
        (
            refusal(io::ErrorKind::InvalidData, "the image cannot be decoded"),
            IpcErrorCode::InvalidInput,
        ),
        (
            refusal(io::ErrorKind::Interrupted, "the read was cancelled"),
            IpcErrorCode::Io,
        ),
        (io::Error::from(io::ErrorKind::NotFound), IpcErrorCode::NotFound),
        (io::Error::from(io::ErrorKind::PermissionDenied), IpcErrorCode::Io),
        // Errors that are not ours: the system's, a library's, a helper that did not start.
        (io::Error::other(format!("cannot open {secret}")), IpcErrorCode::Io),
        (
            io::Error::new(io::ErrorKind::InvalidData, format!("decoder said {secret}")),
            IpcErrorCode::Io,
        ),
        (
            io::Error::new(io::ErrorKind::InvalidInput, format!("bad {secret}")),
            IpcErrorCode::Io,
        ),
        (
            io::Error::new(io::ErrorKind::Unsupported, format!("no {secret}")),
            IpcErrorCode::Io,
        ),
    ];

    for (error, code) in cases {
        let told = thumb_error(&error);
        let json = serde_json::to_value(&told).expect("json");

        assert_eq!(told.code(), code, "{error}");
        assert!(!json["message"].as_str().expect("text").contains(secret), "{error}");
    }
}

/// A file that is a PNG by its first bytes and over the cap, which is too big to be read.
fn too_big_file(folder: &std::path::Path, name: &str) {
    std::fs::write(folder.join(name), PNG_HEAD).expect("a file");
    std::fs::File::options()
        .write(true)
        .open(folder.join(name))
        .expect("open")
        .set_len(MAX_IMAGE_BYTES + 1)
        .expect("a sparse file");
}

#[test]
fn a_file_that_cannot_be_made_into_a_picture_is_remembered_and_not_tried_again() {
    let (folder, path) = folder_with(&[]);
    too_big_file(folder.path(), "huge.png");
    let thumbs = Thumbs::default();

    let first = shown(get(&thumbs, &path, "huge.png", 128));
    assert_eq!(
        first,
        (
            IpcErrorCode::InvalidInput,
            "the image is too big to show here".to_owned()
        )
    );
    assert_eq!(thumbs.cache().bytes(), cache::FAILURE_BYTES, "a small fixed cost");

    // Every place for reading and for the helpers is taken: only what is remembered can answer.
    let _readers = fill(&thumbs.readers, READER_THREADS);
    let _helpers = fill(&thumbs.helpers, HELPER_THREADS);
    for _ in 0..3 {
        assert_eq!(
            shown(get(&thumbs, &path, "huge.png", 128)),
            first,
            "the same code and text"
        );
    }

    // Another size is another try, which finds the places taken.
    assert_eq!(shown(get(&thumbs, &path, "huge.png", 400)).0, IpcErrorCode::Busy);
}

#[test]
fn a_remembered_failure_is_forgotten_when_the_file_changes() {
    let (folder, path) = folder_with(&[]);
    too_big_file(folder.path(), "huge.png");
    let thumbs = Thumbs::default();
    assert_eq!(
        shown(get(&thumbs, &path, "huge.png", 128)).0,
        IpcErrorCode::InvalidInput
    );

    // The file is now a small one: its length differs, so it is tried again (and the helper is not there).
    std::fs::write(folder.path().join("huge.png"), PNG_HEAD).expect("rewritten");

    assert_eq!(shown(get(&thumbs, &path, "huge.png", 128)).0, IpcErrorCode::Io);
    assert_eq!(thumbs.cache().bytes(), 0);
}

#[test]
fn a_failure_of_the_system_is_not_remembered() {
    let (_folder, path) = folder_with(&[("a.png", PNG_HEAD)]);
    let thumbs = Thumbs::default();

    // The helper is not there: not the file's fault, and the next request tries again.
    assert_eq!(shown(get(&thumbs, &path, "a.png", 128)).0, IpcErrorCode::Io);
    assert_eq!(thumbs.cache().bytes(), 0);

    thumbs.shut_down();
    assert_eq!(shown(get(&thumbs, &path, "a.png", 128)).1, "the read was cancelled");
    assert_eq!(thumbs.cache().bytes(), 0, "a cancelled request is not a bad picture");
}

#[test]
fn a_miss_with_every_reader_busy_is_busy_and_a_hit_is_not() {
    let (folder, path) = folder_with(&[("a.png", PNG_HEAD), ("b.png", PNG_HEAD)]);
    let thumbs = Thumbs::default();
    keep(&thumbs, folder.path(), "a.png", 128, picture(b"the picture of a"));
    let _readers = fill(&thumbs.readers, READER_THREADS);

    assert_eq!(shown(get(&thumbs, &path, "b.png", 128)).0, IpcErrorCode::Busy);
    assert!(get(&thumbs, &path, "a.png", 128).is_ok());
}

#[test]
fn a_helper_place_is_not_held_while_the_file_is_read() {
    let (folder, path) = folder_with(&[]);
    too_big_file(folder.path(), "huge.png");
    let thumbs = Thumbs::default();
    // One place of the helpers is free: the request takes the room and reads, and is stopped by the file, not the cap.
    let _helpers = fill_all_but_one(&thumbs.helpers, HELPER_THREADS);

    assert_eq!(
        shown(get(&thumbs, &path, "huge.png", 128)).0,
        IpcErrorCode::InvalidInput
    );
    // And whatever the read took, no place of the helpers is left taken by it.
    assert!(thumbs.helpers.has_room(HELPER_THREADS));
    assert!(thumbs.readers.has_room(1), "nor one of the readers");
}

fn fill_all_but_one(cap: &ThreadCap, max: usize) -> Vec<Permit> {
    (0..max - 1).map(|_| cap.acquire(max).expect("a place")).collect()
}

#[cfg(unix)]
#[test]
fn a_file_that_replaced_another_of_the_same_length_and_time_is_made_again() {
    let (folder, path) = folder_with(&[("a.png", PNG_HEAD), ("new.png", PNG_HEAD)]);
    let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    for name in ["a.png", "new.png"] {
        std::fs::File::options()
            .write(true)
            .open(folder.path().join(name))
            .expect("open")
            .set_modified(time)
            .expect("a time");
    }
    let thumbs = Thumbs::default();
    keep(
        &thumbs,
        folder.path(),
        "a.png",
        128,
        picture(b"the picture of the old file"),
    );
    assert!(get(&thumbs, &path, "a.png", 128).is_ok());

    std::fs::rename(folder.path().join("new.png"), folder.path().join("a.png")).expect("replaced");

    assert_eq!(
        shown(get(&thumbs, &path, "a.png", 128)).0,
        IpcErrorCode::Io,
        "made again, not served"
    );
    assert_eq!(thumbs.cache().bytes(), 0);
}

#[cfg(unix)]
#[test]
fn a_link_is_never_followed_for_a_picture_or_a_key() {
    let (folder, path) = folder_with(&[("a.png", PNG_HEAD)]);
    std::os::unix::fs::symlink("a.png", folder.path().join("link.png")).expect("a link");
    let thumbs = Thumbs::default();
    keep(&thumbs, folder.path(), "a.png", 128, picture(b"the picture of a"));

    // The picture of the file the link leads to is not given for the link.
    let (code, _) = shown(get(&thumbs, &path, "link.png", 128));

    assert_eq!(code, IpcErrorCode::InvalidInput);
    assert!(get(&thumbs, &path, "a.png", 128).is_ok());
}
