//! Which format a file is: by its name, checked against its first bytes, or by the bytes first.

use pretty_assertions::assert_eq;

use crate::model::ArchiveFormat::{self, *};

use super::kind::{by_extension, by_magic, detect};
use super::testing::{gzip, tar_of, zip_of};

const XZ: &[u8] = &[0xFD, b'7', b'z', b'X', b'Z', 0, 0, 4];
const ZSTD: &[u8] = &[0x28, 0xB5, 0x2F, 0xFD, 0, 0];
const BZIP2: &[u8] = b"BZh91AY&SY";
const SEVEN_Z: &[u8] = &[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C, 0, 4];
const RAR4: &[u8] = b"Rar!\x1A\x07\x00\xCF";
const RAR5: &[u8] = b"Rar!\x1A\x07\x01\x00\xCF";

#[test]
fn the_extension_names_the_format() {
    let cases: [(&str, Option<ArchiveFormat>); 20] = [
        ("a.zip", Some(Zip)),
        ("A.JAR", Some(Zip)),
        ("a.tar", Some(Tar)),
        ("a.tar.gz", Some(TarGz)),
        ("a.TGZ", Some(TarGz)),
        ("a.tar.xz", Some(TarXz)),
        ("a.txz", Some(TarXz)),
        ("a.tar.zst", Some(TarZst)),
        ("a.tzst", Some(TarZst)),
        ("a.tar.bz2", Some(TarBz2)),
        ("a.tbz2", Some(TarBz2)),
        ("a.tbz", Some(TarBz2)),
        ("a.7z", Some(SevenZ)),
        ("a.rar", Some(Rar)),
        ("a.gz", Some(Gz)),
        ("a.xz", None),
        ("a.zst", None),
        ("a.bz2", None),
        ("a.txt", None),
        ("tar", None),
    ];

    for (name, format) in cases {
        assert_eq!(by_extension(name), format, "{name}");
    }
}

#[test]
fn a_name_needs_the_bytes_of_its_format_to_count() {
    let tar = tar_of(&[("a.txt", b"hello")]);
    let zip = zip_of(&[("a.txt", b"hello")]);
    let gz = gzip(&tar);

    let cases: [(&str, &[u8], Option<ArchiveFormat>); 16] = [
        ("a.zip", &zip, Some(Zip)),
        ("a.zip", &tar, None),
        ("a.zip", b"", None),
        ("a.tar", &tar, Some(Tar)),
        ("a.tar", &zip, None),
        ("a.tar.gz", &gz, Some(TarGz)),
        ("a.gz", &gz, Some(Gz)),
        ("a.tar.gz", &tar, None),
        ("a.tar.xz", XZ, Some(TarXz)),
        ("a.tar.zst", ZSTD, Some(TarZst)),
        ("a.tar.bz2", BZIP2, Some(TarBz2)),
        ("a.7z", SEVEN_Z, Some(SevenZ)),
        ("a.rar", RAR4, Some(Rar)),
        ("a.rar", RAR5, Some(Rar)),
        ("a.7z", RAR4, None),
        ("a.tar.xz", ZSTD, None),
    ];

    for (name, head, format) in cases {
        assert_eq!(detect(name, &head[..head.len().min(512)]), format, "{name}");
    }
}

#[test]
fn an_old_tar_without_ustar_is_found_by_its_checksum() {
    let mut header = tar::Header::new_old();
    header.set_path("old.txt").expect("a path");
    header.set_size(0);
    header.set_cksum();
    let block = header.as_bytes();

    assert_eq!(detect("old.tar", block), Some(Tar));
    assert_eq!(detect("old.tar", &[1; 512]), None);
    assert_eq!(detect("old.tar", &[0; 512]), None);
}

#[test]
fn the_bytes_come_first_and_the_name_only_tells_a_tar_from_a_lone_compressed_file() {
    let tar = tar_of(&[("a.txt", b"hello")]);

    let cases: [(&[u8], &str, Option<ArchiveFormat>); 13] = [
        (&zip_of(&[]), "anything", Some(Zip)),
        (&tar, "anything", Some(Tar)),
        (SEVEN_Z, "x", Some(SevenZ)),
        (RAR5, "x", Some(Rar)),
        (&gzip(&tar), "a.tar.gz", Some(TarGz)),
        (&gzip(&tar), "a.tgz", Some(TarGz)),
        (&gzip(&tar), "a.gz", Some(Gz)),
        (&gzip(&tar), "a.bin", None),
        (XZ, "a.tar.xz", Some(TarXz)),
        (XZ, "a.xz", None),
        (ZSTD, "a.tzst", Some(TarZst)),
        (BZIP2, "a.tar.bz2", Some(TarBz2)),
        (b"plain text", "a.tar", None),
    ];

    for (head, name, format) in cases {
        assert_eq!(by_magic(head, name), format, "{name}");
    }
}
