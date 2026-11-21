//! Combine jobs on real folders: joining the parts of a split file, and what the `.crc` file checks.

use std::fs;
use std::path::Path;

use pretty_assertions::assert_eq;

use crate::model::{JobPhase, Outcome};

use super::Task;
use super::combine::{CrcFile, parse_crc};
use super::testing::{Ended, Rig, has_no_partial_files, pattern, put, read, task, temp, tree};

const PART: u64 = 2048;

/// Splits `root/src/big.bin` (5000 bytes, in 3 parts) into `root/parts`.
fn split_into_parts(root: &Path) -> Vec<u8> {
    let bytes = pattern(5000);
    put(root, "src/big.bin", &bytes);
    let mut rig = Rig::new();

    let ended = rig.run(task(
        &root.join("src"),
        &["big.bin"],
        &root.join("parts"),
        Task::Split { part_size: PART },
    ));
    assert_eq!(ended.outcome, Outcome::Done);

    bytes
}

/// Joins the parts of `name` in `root/parts` into `root/joined`.
fn join(root: &Path, name: &str) -> (Rig, Ended) {
    let mut rig = Rig::new();

    let ended = rig.run(task(&root.join("parts"), &[name], &root.join("joined"), Task::Combine));

    (rig, ended)
}

fn failure(ended: &Ended) -> String {
    match &ended.outcome {
        Outcome::Failed { error } => error.message.clone(),
        other => panic!("the job fails: {other:?}"),
    }
}

#[test]
fn split_and_combine_make_the_file_again_under_the_name_of_the_crc_file() {
    let root = temp();
    let bytes = split_into_parts(root.path());

    let (rig, ended) = join(root.path(), "big.bin.001");

    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(tree(&root.path().join("joined")), ["big.bin"]);
    assert_eq!(read(&root.path().join("joined/big.bin")), bytes);
    assert!(super::testing::phases(&rig).contains(&JobPhase::Combining));
    assert!(has_no_partial_files(root.path()));
}

#[test]
fn the_name_comes_from_the_crc_file_when_it_is_a_plain_name() {
    let root = temp();
    let bytes = split_into_parts(root.path());
    let crc = root.path().join("parts/big.bin.crc");
    let text = fs::read_to_string(&crc).expect("text");

    for (named, made) in [
        ("renamed.bin", "renamed.bin"),
        ("../escape", "big.bin"),
        ("a/b", "big.bin"),
        (".profile", "big.bin"),
        (".hidden.bin", "big.bin"),
    ] {
        fs::write(&crc, text.replace("filename=big.bin", &format!("filename={named}"))).expect("written");
        let _ = fs::remove_dir_all(root.path().join("joined"));

        let (_, ended) = join(root.path(), "big.bin.001");

        assert_eq!(ended.outcome, Outcome::Done, "{named}");
        assert_eq!(tree(&root.path().join("joined")), [made]);
        assert_eq!(read(&root.path().join("joined").join(made)), bytes);
    }
    assert!(!root.path().join("escape").exists());
}

#[test]
fn the_crc_file_cannot_change_the_kind_of_the_file() {
    let root = temp();
    put(root.path(), "parts/photo.jpg.001", b"ab");
    put(root.path(), "parts/photo.jpg.002", b"cd");
    let crc = crc32fast::hash(b"abcd");

    for (named, made) in [
        ("photo.exe", "photo.jpg"),
        ("photo", "photo.jpg"),
        ("PHOTO.JPG", "PHOTO.JPG"),
        ("other.jpg", "other.jpg"),
    ] {
        let text = format!("filename={named}\r\nsize=4\r\ncrc32={crc:08X}\r\n");
        put(root.path(), "parts/photo.jpg.crc", text.as_bytes());
        let _ = fs::remove_dir_all(root.path().join("joined"));

        let (_, ended) = join(root.path(), "photo.jpg.001");

        assert_eq!(ended.outcome, Outcome::Done, "{named}");
        assert_eq!(tree(&root.path().join("joined")), [made], "{named}");
        assert_eq!(read(&root.path().join("joined").join(made)), b"abcd");
    }
}

#[test]
fn without_a_crc_file_the_parts_are_joined_under_the_name_without_its_number() {
    let root = temp();
    let bytes = split_into_parts(root.path());
    fs::remove_file(root.path().join("parts/big.bin.crc")).expect("removed");

    let (_, ended) = join(root.path(), "big.bin.001");

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(read(&root.path().join("joined/big.bin")), bytes);
}

#[test]
fn a_part_that_was_damaged_fails_the_job_and_no_file_is_kept() {
    let root = temp();
    split_into_parts(root.path());
    let part = root.path().join("parts/big.bin.002");
    let mut bytes = read(&part);
    bytes[10] ^= 0xFF;
    fs::write(&part, bytes).expect("written");

    let (_, ended) = join(root.path(), "big.bin.001");

    assert_eq!(failure(&ended), "the combined file does not match its .crc file");
    assert!(
        !root.path().join("joined").exists(),
        "no file, no partial one, and the folder the job made is removed again"
    );
}

#[test]
fn a_short_part_or_a_missing_one_is_caught_by_the_size_in_the_crc_file() {
    let root = temp();
    split_into_parts(root.path());

    // The last part is cut short.
    let last = root.path().join("parts/big.bin.003");
    fs::write(&last, &read(&last)[..100]).expect("written");
    let (_, ended) = join(root.path(), "big.bin.001");
    assert_eq!(failure(&ended), "the combined file does not match its .crc file");

    // A part in the middle is gone: the join stops there.
    fs::remove_file(root.path().join("parts/big.bin.002")).expect("removed");
    let (_, ended) = join(root.path(), "big.bin.001");
    assert_eq!(failure(&ended), "the combined file does not match its .crc file");
    assert!(!root.path().join("joined").exists());
}

#[cfg(unix)]
#[test]
fn a_part_that_is_a_link_is_refused() {
    let root = temp();
    split_into_parts(root.path());
    let part = root.path().join("parts/big.bin.002");
    let real = root.path().join("real-part");
    fs::rename(&part, &real).expect("moved");
    std::os::unix::fs::symlink(&real, &part).expect("link");

    let (_, ended) = join(root.path(), "big.bin.001");

    assert_eq!(failure(&ended), "a part is not a regular file");
    assert!(!root.path().join("joined/big.bin").exists());
}

#[test]
fn only_the_first_part_can_be_chosen() {
    let root = temp();
    split_into_parts(root.path());

    for name in ["big.bin.002", "big.bin.crc", ".001"] {
        put(root.path(), &format!("parts/{name}"), b"x");
        let (_, ended) = join(root.path(), name);

        assert_eq!(
            failure(&ended),
            "choose the first part of the file: its name ends with .001",
            "{name}"
        );
    }
}

#[test]
fn a_name_that_is_taken_fails_the_job_and_the_file_there_stays() {
    let root = temp();
    split_into_parts(root.path());
    put(root.path(), "joined/big.bin", b"mine");

    let (_, ended) = join(root.path(), "big.bin.001");

    assert_eq!(failure(&ended), "something with this name exists already");
    assert_eq!(read(&root.path().join("joined/big.bin")), b"mine");
    assert_eq!(tree(&root.path().join("joined")), ["big.bin"]);
}

#[test]
fn a_crc_file_that_says_nothing_usable_fails_the_job() {
    let root = temp();
    split_into_parts(root.path());

    for text in ["", "nothing here\r\n", "size=abc\r\n", "crc32=xyz\r\n"] {
        fs::write(root.path().join("parts/big.bin.crc"), text).expect("written");

        let (_, ended) = join(root.path(), "big.bin.001");

        assert_eq!(failure(&ended), "the .crc file cannot be read", "{text:?}");
    }
}

#[test]
fn a_cancel_while_joining_removes_the_partial_file() {
    let root = temp();
    split_into_parts(root.path());
    let mut rig = Rig::paused_in(JobPhase::Combining);

    let job = rig.start(task(
        &root.path().join("parts"),
        &["big.bin.001"],
        &root.path().join("joined"),
        Task::Combine,
    ));

    assert_eq!(rig.cancel_when_paused(job).outcome, Outcome::Cancelled);
    assert!(!root.path().join("joined").exists());
    assert!(has_no_partial_files(root.path()));
}

#[test]
fn a_crc_file_is_read_by_its_keys() {
    assert_eq!(
        parse_crc("filename=a b.bin\r\nsize=12\r\ncrc32=00AbCdEf\r\n"),
        Some(CrcFile {
            filename: Some("a b.bin".to_owned()),
            size: Some(12),
            crc32: Some(0x00AB_CDEF),
        })
    );
    assert_eq!(
        parse_crc("SIZE = 5\n"),
        Some(CrcFile {
            filename: None,
            size: Some(5),
            crc32: None
        })
    );
    assert_eq!(parse_crc("filename=only.bin\n"), None);
}
