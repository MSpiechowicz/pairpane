//! The parent's end of a thumbnail: what is refused before a helper is started, and what is not believed of one. The
//! helper itself, as a real process, is tried by `src-tauri/tests/thumb_helper.rs`.

use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use image::ImageFormat::Png;
use pretty_assertions::assert_eq;

use super::fixtures::{encode, size_of, solid};
use super::{make, open};
use crate::view::heic::HEADER;
use crate::view::heic::process::read_picture;
use crate::view::testing::{idle, put, temp};
use crate::view::{MAX_IMAGE_BYTES, is_refusal};

/// Where a helper would be; what must not start one never reaches it.
const NOWHERE: &str = "/nonexistent/pairpane-helper";

/// An answer of a picture of `width` by `height`, its pixels set to 7.
fn answer(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![0];
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.resize(1 + HEADER + (width * height * 4) as usize, 7);

    bytes
}

#[test]
fn a_file_that_is_not_a_picture_is_refused_at_once_whatever_its_name() {
    let folder = temp();
    put(folder.path(), "notes.txt", b"hello");
    put(folder.path(), "photo.png", b"hello, not a picture at all");
    put(folder.path(), "movie.mp4", b"\0\0\0\x18ftypmp42\0\0\0\0mp42isom");
    put(folder.path(), "empty.jpg", b"");

    for name in ["notes.txt", "photo.png", "movie.mp4", "empty.jpg"] {
        let error = open(folder.path(), name).expect_err(name);

        assert_eq!(error.kind(), io::ErrorKind::Unsupported, "{name}");
        assert!(is_refusal(&error), "{name}");
        assert_eq!(error.to_string(), "the file is not a picture that has a thumbnail");
    }
}

#[test]
fn a_picture_is_known_by_its_first_bytes_and_not_by_its_name() {
    let folder = temp();
    put(folder.path(), "photo.txt", &encode(Png, 4, 4));
    put(folder.path(), "photo", &encode(Png, 4, 4));

    for name in ["photo.txt", "photo"] {
        let source = open(folder.path(), name).expect(name);

        assert_eq!(source.len, encode(Png, 4, 4).len() as u64);
        assert!(!source.is_heic());
    }
}

#[test]
fn a_heic_file_is_known_by_its_brands() {
    let folder = temp();
    put(folder.path(), "a.jpg", b"\0\0\0\x18ftypheic\0\0\0\0mif1heic\0\0\0\0");

    assert!(open(folder.path(), "a.jpg").expect("a picture").is_heic());
}

#[test]
fn only_a_regular_file_is_opened() {
    let folder = temp();
    std::fs::create_dir(folder.path().join("dir.png")).expect("a folder");

    let error = open(folder.path(), "dir.png").expect_err("a folder");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(is_refusal(&error));

    let error = open(folder.path(), "../x.png").expect_err("not a plain name");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);

    let error = open(folder.path(), "missing.png").expect_err("missing");
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}

#[cfg(unix)]
#[test]
fn a_link_is_refused() {
    let folder = temp();
    put(folder.path(), "real.png", &encode(Png, 4, 4));
    std::os::unix::fs::symlink("real.png", folder.path().join("link.png")).expect("a link");

    let error = open(folder.path(), "link.png").expect_err("a link");

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "a thumbnail is made of a regular file, not a link");
}

#[test]
fn a_file_over_the_cap_is_too_big_without_being_read_or_a_helper_started() {
    let folder = temp();
    let path = folder.path().join("huge.png");
    let mut bytes = encode(Png, 4, 4);
    bytes.truncate(64);
    put(folder.path(), "huge.png", &bytes);
    // A sparse file that is only a PNG header and then holes: reading it whole would take a while and 64 MiB.
    File::options()
        .write(true)
        .open(&path)
        .expect("open")
        .set_len(MAX_IMAGE_BYTES + 1)
        .expect("sparse");
    let source = open(folder.path(), "huge.png").expect("a picture by its header");

    let error = make(&source, 64, &idle(), Path::new(NOWHERE)).expect_err("too big");

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(is_refusal(&error), "refused by us, not a helper that failed to start");
    assert_eq!(error.to_string(), "the image is too big to show here");
}

#[test]
fn a_flag_raised_before_the_start_starts_nothing() {
    let folder = temp();
    put(folder.path(), "a.png", &encode(Png, 4, 4));
    let source = open(folder.path(), "a.png").expect("a picture");
    let cancel = AtomicBool::new(true);

    let error = make(&source, 64, &cancel, Path::new(NOWHERE)).expect_err("cancelled");

    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
}

#[test]
fn a_helper_that_is_not_there_is_a_failure_without_text_of_ours() {
    let folder = temp();
    put(folder.path(), "a.png", &encode(Png, 4, 4));
    let source = open(folder.path(), "a.png").expect("a picture");

    let error = make(&source, 64, &idle(), Path::new(NOWHERE)).expect_err("no helper");

    assert!(!is_refusal(&error));
    assert_ne!(error.kind(), io::ErrorKind::NotFound, "not mistaken for a missing file");
}

#[test]
fn the_parent_takes_a_picture_that_fits_and_not_one_that_is_bigger_than_asked() {
    let fits = answer(64, 32);
    assert_eq!(
        read_picture(&fits[..], 64).expect("a picture").len(),
        HEADER + 64 * 32 * 4
    );

    // A helper that answers with a bigger picture than the side it was given is lying, or broken.
    for bigger in [answer(65, 1), answer(1, 65), answer(200, 200)] {
        let error = read_picture(&bigger[..], 64).expect_err("refused");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(is_refusal(&error));
    }
}

#[test]
fn the_picture_of_an_answer_is_framed_as_the_helper_frames_it() {
    let image = solid(3, 2, [1, 2, 3, 4]);
    let framed = super::format::frame(3, 2, image.as_raw());

    assert_eq!(size_of(&framed), (3, 2));
    let mut answer = vec![0];
    answer.extend_from_slice(&framed);
    assert_eq!(read_picture(&answer[..], 64).expect("a picture"), framed);
}
