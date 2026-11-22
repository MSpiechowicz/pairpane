//! The open file: reads at offsets, after a rename, past the end, and in a sparse file of several gigabytes.

use std::fs;
use std::io::{Read, Seek, SeekFrom};

use pretty_assertions::assert_eq;

use crate::archive::FileReader;

use super::testing::{file_of, opened, temp};
use super::{MAX_READ, open};

#[test]
fn a_view_file_can_be_shared_between_threads() {
    fn shared<T: Send + Sync>() {}

    shared::<super::ViewFile>();
}

#[test]
fn reads_at_an_offset_and_stops_at_the_end() {
    let (_folder, file) = file_of(b"0123456789");

    assert_eq!(file.read_at(2, 3).expect("read"), b"234");
    assert_eq!(file.read_at(8, 100).expect("read"), b"89");
    assert_eq!(file.read_at(10, 5).expect("read"), b"");
    assert_eq!(file.read_at(u64::MAX, 5).expect("read"), b"");
    assert_eq!(file.read_at(0, 0).expect("read"), b"");
    assert_eq!(file.size().expect("size"), 10);
}

#[test]
fn refuses_a_read_over_the_cap_with_a_fixed_text() {
    let (_folder, file) = file_of(b"abc");

    let error = file.read_at(0, MAX_READ + 1).expect_err("too long");

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "the read is too long");
    assert_eq!(file.read_at(0, MAX_READ).expect("at the cap").len(), 3);
}

#[test]
fn keeps_reading_after_the_file_is_renamed_or_deleted() {
    let (folder, opened) = opened("a.txt", b"still here");

    fs::rename(folder.path().join("a.txt"), folder.path().join("b.txt")).expect("rename");
    assert_eq!(opened.file.read_at(0, 100).expect("read"), b"still here");

    fs::remove_file(folder.path().join("b.txt")).expect("remove");
    assert_eq!(opened.file.read_at(6, 4).expect("read"), b"here");
}

#[test]
fn the_size_follows_the_file() {
    let folder = temp();
    let path = folder.path().join("grow.txt");
    fs::write(&path, b"abc").expect("write");
    let opened = open(folder.path(), "grow.txt").expect("open");

    fs::write(&path, b"abcdef").expect("grow");

    assert_eq!(opened.file.size().expect("size"), 6);
    assert_eq!(opened.file.read_at(0, 10).expect("read"), b"abcdef");
}

#[test]
fn the_reader_reads_and_seeks_on_its_own_position() {
    let (_folder, file) = file_of(b"0123456789");
    let mut reader = FileReader::new(&file);
    let mut four = [0; 4];

    reader.read_exact(&mut four).expect("read");
    assert_eq!(&four, b"0123");

    assert_eq!(reader.seek(SeekFrom::Current(2)).expect("seek"), 6);
    assert_eq!(reader.seek(SeekFrom::End(-1)).expect("seek"), 9);
    let mut rest = Vec::new();
    reader.read_to_end(&mut rest).expect("read");
    assert_eq!(rest, b"9");

    assert_eq!(reader.seek(SeekFrom::Start(3)).expect("seek"), 3);
    assert!(reader.seek(SeekFrom::Current(-4)).is_err());
    assert!(reader.seek(SeekFrom::End(-11)).is_err());
}

#[cfg(unix)]
#[test]
fn reads_the_end_of_a_sparse_file_of_three_gigabytes() {
    use std::os::unix::fs::FileExt;

    let folder = temp();
    let size: u64 = 3 * 1024 * 1024 * 1024;
    let handle = fs::File::create(folder.path().join("big.bin")).expect("create");
    handle.set_len(size).expect("length");
    handle.write_all_at(b"the end", size - 7).expect("write");
    drop(handle);

    let opened = open(folder.path(), "big.bin").expect("opens at once");

    assert_eq!(opened.size, size);
    assert_eq!(opened.file.size().expect("size"), size);
    assert_eq!(opened.file.read_at(size - 7, 100).expect("read"), b"the end");
    assert_eq!(opened.file.read_at(size - 20, 13).expect("read"), vec![0; 13]);
}
