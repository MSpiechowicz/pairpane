//! What `columns_dimensions` and `columns_checksum` check before a thread is taken, the caps, the answers in the order of
//! the names, and what the page is told. The header parsers themselves are tried in the core crate.

use pairpane_core::view::columns::MAX_CHECKSUM_BYTES;
use tauri::async_runtime::block_on;

use super::*;
use crate::commands::limit::Permit;
use crate::commands::testing::{folder_with, names};

/// The first bytes of a PNG of `width` by `height` pixels.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0]);

    bytes
}

const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn fill(cap: &ThreadCap, max: usize) -> Vec<Permit> {
    (0..max).map(|_| cap.acquire(max).expect("a place")).collect()
}

fn sizes(columns: &Columns, path: &str, list: &[&str]) -> CommandResult<Vec<Option<Dimensions>>> {
    block_on(columns.dimensions(path.to_owned(), names(list)))
}

fn hash(columns: &Columns, path: &str, name: &str) -> CommandResult<String> {
    block_on(columns.checksum(path.to_owned(), name.to_owned()))
}

/// The code and the text the page is told of a refusal.
fn shown<T>(result: CommandResult<T>) -> (IpcErrorCode, String) {
    let Err(error) = result else {
        panic!("refused");
    };
    let json = serde_json::to_value(&error).expect("json");

    (error.code(), json["message"].as_str().expect("message").to_owned())
}

fn size(width: u32, height: u32) -> Option<Dimensions> {
    Some(Dimensions { width, height })
}

#[test]
fn the_sizes_come_back_in_the_order_of_the_names_with_none_for_what_is_no_picture() {
    let (folder, path) = folder_with(&[
        ("a.png", &png(640, 480)),
        ("b.png", &png(3, 4)),
        ("notes.txt", b"hello"),
        ("fake.png", b"this is not a png"),
    ]);
    std::fs::create_dir(folder.path().join("dir.png")).expect("a folder");
    let columns = Columns::default();

    let answer = sizes(
        &columns,
        &path,
        &[
            "b.png",
            "notes.txt",
            "missing.png",
            "a.png",
            "dir.png",
            "fake.png",
            "a.png",
        ],
    )
    .expect("sizes");

    assert_eq!(
        answer,
        vec![size(3, 4), None, None, size(640, 480), None, None, size(640, 480)]
    );
}

#[test]
fn no_names_is_no_sizes() {
    let (_folder, path) = folder_with(&[]);

    assert_eq!(sizes(&Columns::default(), &path, &[]).expect("sizes"), vec![]);
}

#[cfg(unix)]
#[test]
fn a_link_is_not_followed_for_a_size_or_a_checksum() {
    let (folder, path) = folder_with(&[("a.png", &png(640, 480))]);
    std::os::unix::fs::symlink("a.png", folder.path().join("link.png")).expect("a link");
    let columns = Columns::default();

    assert_eq!(
        sizes(&columns, &path, &["link.png", "a.png"]).expect("sizes"),
        vec![None, size(640, 480)]
    );

    let (code, message) = shown(hash(&columns, &path, "link.png"));
    assert_eq!(code, IpcErrorCode::InvalidInput);
    assert!(!message.contains(&path), "{message}");
}

#[test]
fn at_most_512_names_are_taken_in_one_call() {
    let (_folder, path) = folder_with(&[]);
    let columns = Columns::default();
    let many: Vec<String> = (0..513).map(|index| format!("{index}.png")).collect();
    let all: Vec<&str> = many.iter().map(String::as_str).collect();

    assert_eq!(sizes(&columns, &path, &all[..512]).expect("512 names").len(), 512);

    let (code, message) = shown(sizes(&columns, &path, &all));
    assert_eq!(code, IpcErrorCode::InvalidInput);
    assert_eq!(message, "ask for at most 512 names at a time");
}

#[test]
fn what_the_page_sent_is_checked_before_a_thread_is_taken() {
    let (_folder, path) = folder_with(&[("a.png", &png(1, 1))]);
    let columns = Columns::default();
    let _full = (fill(&columns.sizes, SIZE_THREADS), fill(&columns.hashes, HASH_THREADS));

    for name in ["../a.png", "sub/a.png", "..", "", "a/../a.png"] {
        assert_eq!(
            shown(hash(&columns, &path, name)).0,
            IpcErrorCode::InvalidInput,
            "{name:?}"
        );
    }

    assert_eq!(
        shown(hash(&columns, &path, &"x".repeat(5000))).0,
        IpcErrorCode::InvalidInput
    );
    assert_eq!(
        shown(sizes(&columns, "relative/folder", &["a.png"])).0,
        IpcErrorCode::InvalidInput
    );
    assert_eq!(
        shown(hash(&columns, "relative/folder", "a.png")).0,
        IpcErrorCode::InvalidInput
    );

    // A request that passes the checks is stopped by the full caps, which is a different answer.
    assert_eq!(shown(sizes(&columns, &path, &["a.png"])).0, IpcErrorCode::Busy);
    assert_eq!(shown(hash(&columns, &path, "a.png")).0, IpcErrorCode::Busy);
}

#[test]
fn a_name_of_a_batch_that_is_not_a_plain_name_has_none_in_its_place_and_the_others_are_read() {
    let (_folder, path) = folder_with(&[("a.png", &png(640, 480)), ("b.png", &png(3, 4))]);
    let long = "x".repeat(5000);

    let answer = sizes(
        &Columns::default(),
        &path,
        &["a.png", "../a.png", "sub/a.png", "..", "", &long, "b.png"],
    )
    .expect("sizes");

    assert_eq!(answer, vec![size(640, 480), None, None, None, None, None, size(3, 4)]);
}

#[test]
fn a_full_cap_refuses_as_busy_and_a_freed_place_is_used() {
    let (_folder, path) = folder_with(&[("a.png", &png(2, 3)), ("abc.txt", b"abc")]);
    let columns = Columns::default();

    let batches = fill(&columns.sizes, SIZE_THREADS);
    assert_eq!(shown(sizes(&columns, &path, &["a.png"])).0, IpcErrorCode::Busy);
    drop(batches);
    assert_eq!(sizes(&columns, &path, &["a.png"]).expect("sizes"), vec![size(2, 3)]);

    let hashes = fill(&columns.hashes, HASH_THREADS);
    assert_eq!(shown(hash(&columns, &path, "abc.txt")).0, IpcErrorCode::Busy);
    drop(hashes);
    assert_eq!(hash(&columns, &path, "abc.txt").expect("hash"), ABC_SHA256);
}

#[test]
fn the_checksum_is_the_sha256_of_the_file_as_lowercase_hex() {
    let (_folder, path) = folder_with(&[("abc.txt", b"abc"), ("empty", b"")]);
    let columns = Columns::default();

    assert_eq!(hash(&columns, &path, "abc.txt").expect("hash"), ABC_SHA256);
    assert_eq!(
        hash(&columns, &path, "empty").expect("hash"),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn a_file_over_the_cap_is_refused_with_a_fixed_text() {
    let (folder, path) = folder_with(&[]);
    let over = std::fs::File::create(folder.path().join("over.bin")).expect("file");
    over.set_len(MAX_CHECKSUM_BYTES + 1).expect("sparse");

    let (code, message) = shown(hash(&Columns::default(), &path, "over.bin"));

    assert_eq!(code, IpcErrorCode::InvalidInput);
    assert_eq!(message, "the file is too big to checksum here");
}

#[test]
fn no_text_sent_to_the_page_holds_a_path() {
    let (folder, path) = folder_with(&[]);
    std::fs::create_dir(folder.path().join("dir")).expect("a folder");
    let columns = Columns::default();

    let (missing, missing_text) = shown(hash(&columns, &path, "missing.txt"));
    assert_eq!(missing, IpcErrorCode::NotFound);

    let (folder_code, folder_text) = shown(hash(&columns, &path, "dir"));
    assert_eq!(folder_code, IpcErrorCode::InvalidInput);

    let (_, relative_text) = shown(hash(&columns, "relative/folder", "a"));
    let (_, many_text) = shown(sizes(&columns, &path, &vec!["a"; 600]));

    for text in [missing_text, folder_text, relative_text, many_text] {
        assert!(!text.contains(&path) && !text.contains("relative"), "{text}");
    }
}

#[test]
fn after_the_app_is_told_to_exit_nothing_is_read() {
    let (_folder, path) = folder_with(&[("a.png", &png(2, 3)), ("abc.txt", b"abc")]);
    let columns = Columns::default();
    columns.shut_down();

    let (code, message) = shown(hash(&columns, &path, "abc.txt"));
    assert_eq!((code, message.as_str()), (IpcErrorCode::Io, "the read was cancelled"));

    let (code, message) = shown(sizes(&columns, &path, &["a.png"]));
    assert_eq!((code, message.as_str()), (IpcErrorCode::Io, "the read was cancelled"));

    assert_eq!(sizes(&columns, &path, &[]).expect("nothing to read"), vec![]);
}

#[test]
fn the_places_are_given_back_when_a_call_ends_so_that_many_calls_in_a_row_are_served() {
    let (_folder, path) = folder_with(&[("a.png", &png(2, 3)), ("abc.txt", b"abc")]);
    let columns = Columns::default();

    for _ in 0..(SIZE_THREADS + HASH_THREADS) * 3 {
        assert_eq!(sizes(&columns, &path, &["a.png"]).expect("sizes"), vec![size(2, 3)]);
        assert_eq!(hash(&columns, &path, "abc.txt").expect("hash"), ABC_SHA256);
    }
}
