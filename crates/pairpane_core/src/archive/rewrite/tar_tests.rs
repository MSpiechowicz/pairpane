//! Rewriting a tar in each of its compressions: what stays is the same blocks (the headers that describe an entry with
//! it), what is left out takes its describing headers along, and a header that is too large or a stream that is damaged
//! fails the rewrite.

use pretty_assertions::assert_eq;
use tar::{Builder, EntryType, Header};

use crate::archive::write::testing::{Item, sample};
use crate::model::ArchiveFormat;

use super::testing::{TARS, archive, changed, index, ordinal, rewrite_with, seen, wanted};

fn added() -> Vec<Item> {
    vec![
        Item::folder("fresh", 0o755),
        Item::file("fresh/new.txt", 0o644, b"brand new"),
        Item::file("src/extra.rs", 0o600, b"pub fn extra() {}\n"),
    ]
}

/// A PAX record of a header: its length (which counts itself), the key and the value.
fn pax(key: &str, value: &[u8]) -> Vec<u8> {
    let body = key.len() + value.len() + 3;
    let mut length = body + body.to_string().len();
    if length.to_string().len() != body.to_string().len() {
        length += 1;
    }

    let mut record = format!("{length} {key}=").into_bytes();
    record.extend_from_slice(value);
    record.push(b'\n');

    record
}

/// A tar whose entries have their paths in PAX headers: `(path in the header block, path in PAX, content)`.
fn pax_tar(entries: &[(&str, &str, &[u8])], records: &[u8]) -> Vec<u8> {
    let mut builder = Builder::new(Vec::new());

    for (short, long, content) in entries {
        let data = [records, &pax("path", long.as_bytes())].concat();
        let mut extended = Header::new_ustar();
        extended.set_entry_type(EntryType::XHeader);
        extended.set_size(data.len() as u64);
        extended.set_cksum();
        builder
            .append_data(&mut extended, "PaxHeader/x", data.as_slice())
            .expect("pax");

        let mut header = Header::new_ustar();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        builder.append_data(&mut header, short, *content).expect("entry");
    }

    builder.into_inner().expect("a tar")
}

#[test]
fn every_codec_reads_back_what_stays_what_is_added_and_not_what_is_left_out() {
    let items = sample();
    let long = items
        .iter()
        .map(|item| item.name.clone())
        .find(|name| name.len() > 200)
        .expect("a long name");

    for format in TARS {
        let old = archive(format, &items, None, false);
        let gone = [ordinal(&old, format, "src/main.rs"), ordinal(&old, format, &long)];

        let new = rewrite_with(&old, format, (&gone, &added()), None).expect("rewritten");

        assert_eq!(
            seen(&new, format, None),
            wanted(format, &changed(&items, &["src/main.rs", &long], &added())),
            "{format:?}"
        );
    }
}

#[test]
fn a_plain_tar_with_nothing_changed_is_the_same_bytes() {
    let old = archive(ArchiveFormat::Tar, &sample(), None, false);

    assert_eq!(
        rewrite_with(&old, ArchiveFormat::Tar, (&[], &[]), None).expect("rewritten"),
        old
    );
}

#[test]
fn the_headers_of_an_entry_that_is_left_out_go_with_it_and_those_of_the_ones_that_stay_do_not() {
    let old = pax_tar(
        &[("one", "pax/path/one", b"first"), ("two", "pax/path/two", b"second")],
        b"",
    );
    let format = ArchiveFormat::Tar;
    let first = ordinal(&old, format, "pax/path/one");

    let new = rewrite_with(&old, format, (&[first], &added()), None).expect("rewritten");

    let paths: Vec<String> = seen(&new, format, None).into_iter().map(|seen| seen.path).collect();
    assert_eq!(paths, ["fresh", "fresh/new.txt", "pax/path/two", "src/extra.rs"]);
    // The entry that stays is what it was: its PAX header, its header block, its data.
    let (kept, _) = old.split_at(old.len() - 1024);
    let second = kept.len() - 4 * 512;
    assert!(new.windows(kept.len() - second).any(|window| window == &kept[second..]));
}

#[test]
fn a_header_larger_than_what_is_kept_fails_the_rewrite_with_a_fixed_text() {
    let huge = pax("comment", &vec![b'x'; 300_000]);
    let old = pax_tar(&[("one", "pax/path/one", b"first")], &huge);

    let error = rewrite_with(&old, ArchiveFormat::Tar, (&[], &[]), None).expect_err("refused");

    assert_eq!(error.to_string(), "a header of the archive is too large to be kept");
}

#[test]
fn a_compressed_tar_whose_stream_is_damaged_is_not_carried_over() {
    let mut old = archive(ArchiveFormat::TarGz, &sample(), None, false);
    // The gzip trailer: the checksum of the whole tar, which is only read after the last entry.
    let at = old.len() - 8;
    old[at] ^= 0xff;
    assert!(index(&old, ArchiveFormat::TarGz, None).is_ok());

    assert!(rewrite_with(&old, ArchiveFormat::TarGz, (&[], &added()), None).is_err());
}

const AFTER_END: &str = "the archive has data after its end, so it can't be changed";

/// A tar with a PAX global header of the records `records`, then an entry.
fn global_tar(records: &[u8]) -> Vec<u8> {
    let mut builder = Builder::new(Vec::new());

    let mut global = Header::new_ustar();
    global.set_entry_type(EntryType::XGlobalHeader);
    global.set_size(records.len() as u64);
    global.set_cksum();
    builder
        .append_data(&mut global, "pax_global_header", records)
        .expect("global");

    let mut header = Header::new_ustar();
    header.set_size(5);
    header.set_mode(0o644);
    builder.append_data(&mut header, "one", &b"first"[..]).expect("entry");

    builder.into_inner().expect("a tar")
}

fn refused_with(bytes: &Vec<u8>, format: ArchiveFormat, text: &str) {
    let error = rewrite_with(bytes, format, (&[], &added()), None).expect_err("refused");

    assert_eq!(error.kind(), std::io::ErrorKind::Unsupported, "{format:?}");
    assert_eq!(error.to_string(), text, "{format:?}");
}

#[test]
fn a_tar_that_goes_on_after_its_end_blocks_is_refused_in_every_codec() {
    let first = sample();
    let second = [Item::file("other/second.txt", 0o644, b"second tar")];

    for format in TARS {
        // What `cat a.tar b.tar` makes (and the same of two compressed files): the second tar is not listed, so a rewrite
        // would drop it.
        let mut both = archive(format, &first, None, false);
        both.extend(archive(format, &second, None, false));

        refused_with(&both, format, AFTER_END);
    }
}

#[test]
fn bytes_after_the_end_of_a_plain_tar_or_of_the_first_gzip_member_are_refused() {
    for format in [ArchiveFormat::Tar, ArchiveFormat::TarGz] {
        let mut old = archive(format, &sample(), None, false);
        old.extend(b"anything but zeros, as a footer of some other program");

        refused_with(&old, format, AFTER_END);
    }
}

#[test]
fn zeros_after_the_end_blocks_are_what_a_tar_may_have() {
    let mut old = archive(ArchiveFormat::Tar, &sample(), None, false);
    // The padding of a tar to a whole record.
    old.resize(old.len() + 10_240, 0);

    let new = rewrite_with(&old, ArchiveFormat::Tar, (&[], &added()), None).expect("rewritten");

    assert_eq!(
        seen(&new, ArchiveFormat::Tar, None),
        wanted(ArchiveFormat::Tar, &changed(&sample(), &[], &added()))
    );
}

#[test]
fn a_pax_global_header_with_settings_refuses_an_add_but_not_a_delete_and_a_comment_is_no_setting() {
    let format = ArchiveFormat::Tar;
    let settings = global_tar(&[pax("mtime", b"0"), pax("comment", b"hi")].concat());
    refused_with(
        &settings,
        format,
        "this archive has settings that would change the added files, so it can't be changed",
    );
    let first = ordinal(&settings, format, "one");
    assert!(rewrite_with(&settings, format, (&[first], &[]), None).is_ok());

    let notes = global_tar(&[pax("comment", b"hi"), pax("charset", b"BINARY")].concat());
    let new = rewrite_with(&notes, format, (&[], &added()), None).expect("rewritten");
    assert_eq!(seen(&new, format, None).len(), 4);
}
