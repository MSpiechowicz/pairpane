//! `dimensions` and `checksum` on files of a folder: what is read, what is refused and that nothing is followed.

use std::sync::atomic::{AtomicBool, Ordering};

use pretty_assertions::assert_eq;

use crate::model::Dimensions;

use super::fixtures::{jpeg_of, png, segment};
use super::{HEAD_BYTES, MAX_CHECKSUM_BYTES, checksum, dimensions};

fn folder_with(files: &[(&str, &[u8])]) -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("folder");
    for (name, bytes) in files {
        std::fs::write(folder.path().join(name), bytes).expect("file");
    }

    folder
}

const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

#[test]
fn the_size_of_a_picture_file_is_read_by_its_first_bytes_whatever_its_name() {
    let folder = folder_with(&[("photo.txt", &png(640, 480)), ("notes.png", b"not a picture")]);

    assert_eq!(
        dimensions(folder.path(), "photo.txt"),
        Some(Dimensions {
            width: 640,
            height: 480
        })
    );
    assert_eq!(dimensions(folder.path(), "notes.png"), None);
}

#[test]
fn a_jpeg_with_its_frame_past_the_first_64_kib_of_a_big_file_has_no_dimensions_and_one_inside_has() {
    let padding: Vec<Vec<u8>> = (0..20).map(|_| segment(0xE2, &[0; 4000])).collect();
    let mut far = jpeg_of(&padding, 640, 480);
    far.resize(10 * HEAD_BYTES, 0);
    let near = jpeg_of(&padding[..5], 640, 480);
    let folder = folder_with(&[("far.jpg", &far), ("near.jpg", &near)]);

    assert_eq!(dimensions(folder.path(), "far.jpg"), None);
    assert_eq!(
        dimensions(folder.path(), "near.jpg"),
        Some(Dimensions {
            width: 640,
            height: 480
        })
    );
}

#[test]
fn a_missing_file_a_folder_and_a_name_that_is_not_plain_have_no_dimensions() {
    let folder = folder_with(&[("a.png", &png(1, 1))]);
    std::fs::create_dir(folder.path().join("dir.png")).expect("folder");

    for name in ["missing.png", "dir.png", "../a.png", "sub/a.png", "..", ""] {
        assert_eq!(dimensions(folder.path(), name), None, "{name:?}");
    }
}

#[cfg(unix)]
#[test]
fn a_link_is_not_followed_for_the_size_or_the_checksum() {
    use crate::view::is_refusal;

    let folder = folder_with(&[("a.png", &png(1, 1))]);
    std::os::unix::fs::symlink("a.png", folder.path().join("link.png")).expect("link");

    assert_eq!(dimensions(folder.path(), "link.png"), None);

    let error = checksum(folder.path(), "link.png", &AtomicBool::new(false)).expect_err("a link");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(is_refusal(&error), "{error}");
}

#[test]
fn the_checksum_is_the_sha256_of_the_file_as_lowercase_hex() {
    let folder = folder_with(&[("abc.txt", b"abc"), ("empty", b"")]);
    let cancel = AtomicBool::new(false);

    assert_eq!(checksum(folder.path(), "abc.txt", &cancel).expect("hash"), ABC_SHA256);
    assert_eq!(
        checksum(folder.path(), "empty", &cancel).expect("hash"),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn a_file_read_in_several_chunks_has_the_hash_of_all_of_it() {
    // 2.5 MiB of one byte: more than one chunk, and a last one that is short.
    let bytes = vec![b'a'; 2 * 1024 * 1024 + 512 * 1024];
    let folder = folder_with(&[("big", &bytes)]);

    // The reference is the hash crate on the whole buffer at once, not the chunked loop.
    let expected = {
        use sha2::{Digest, Sha256};

        let sum = Sha256::digest(&bytes);
        sum.iter().map(|byte| format!("{byte:02x}")).collect::<String>()
    };

    assert_eq!(
        checksum(folder.path(), "big", &AtomicBool::new(false)).expect("hash"),
        expected
    );
}

#[test]
fn a_file_over_the_cap_is_refused_without_being_read_and_one_at_the_cap_is_taken() {
    let folder = tempfile::tempdir().expect("folder");
    let over = std::fs::File::create(folder.path().join("over")).expect("file");
    over.set_len(MAX_CHECKSUM_BYTES + 1).expect("sparse");
    let at = std::fs::File::create(folder.path().join("at")).expect("file");
    at.set_len(MAX_CHECKSUM_BYTES).expect("sparse");
    let cancel = AtomicBool::new(false);

    let error = checksum(folder.path(), "over", &cancel).expect_err("too big");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "the file is too big to checksum here");

    assert_eq!(checksum(folder.path(), "at", &cancel).expect("at the cap").len(), 64);
}

#[test]
fn a_raised_cancel_flag_stops_the_checksum_and_a_folder_or_missing_name_is_refused() {
    let folder = folder_with(&[("abc.txt", b"abc")]);
    std::fs::create_dir(folder.path().join("dir")).expect("folder");
    let cancel = AtomicBool::new(true);

    let stopped = checksum(folder.path(), "abc.txt", &cancel).expect_err("stopped");
    assert_eq!(stopped.kind(), std::io::ErrorKind::Interrupted);

    cancel.store(false, Ordering::Relaxed);
    let folder_error = checksum(folder.path(), "dir", &cancel).expect_err("a folder");
    assert_eq!(folder_error.kind(), std::io::ErrorKind::InvalidInput);
    let missing = checksum(folder.path(), "missing", &cancel).expect_err("missing");
    assert_eq!(missing.kind(), std::io::ErrorKind::NotFound);

    // The text of what we refuse holds no path.
    for error in [stopped, folder_error] {
        assert!(
            !error.to_string().contains(&*folder.path().to_string_lossy()),
            "{error}"
        );
    }
}
