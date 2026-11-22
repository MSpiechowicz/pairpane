//! The origin marker of an archive and its copy onto what is extracted. Where the platform has no marker (Linux) the
//! tests show that nothing changes; the stream (Windows) and the attribute (macOS) are tried on their own systems.

use std::fs;

use pretty_assertions::assert_eq;

use super::origin::Origin;
use super::testing::{read, temp};

/// An archive `a.zip` and a written file `out.txt` in `root`.
fn archive_and_file(root: &tempfile::TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let archive = root.path().join("a.zip");
    let file = root.path().join("out.txt");
    fs::write(&archive, b"zip").expect("archive");
    fs::write(&file, b"data").expect("file");

    (archive, file)
}

#[test]
fn an_archive_without_a_marker_leaves_the_files_as_they_are() {
    let root = temp();
    let (archive, file) = archive_and_file(&root);

    let origin = Origin::of(&archive);
    origin.apply(&file);

    assert_eq!(origin.bytes(), None);
    assert_eq!(read(&file), b"data");
}

#[test]
fn a_file_that_is_not_there_has_no_marker_and_taking_one_to_a_missing_file_fails_quietly() {
    let root = temp();

    let origin = Origin::of(&root.path().join("missing.zip"));
    origin.apply(&root.path().join("missing.txt"));

    assert_eq!(origin.bytes(), None);
}

/// The attribute calls the macOS marker is made of, tried on a `user.` attribute where Linux has them.
#[cfg(target_os = "linux")]
#[test]
fn an_extended_attribute_is_copied_from_one_file_to_another() {
    use super::origin::attribute::{read_attribute, write_attribute};

    let root = temp();
    let from = root.path().join("from");
    let to = root.path().join("to");
    fs::write(&from, b"a").expect("file");
    fs::write(&to, b"b").expect("file");

    write_attribute(&from, "user.pairpane.origin", b"0083;5f3c;Safari;");
    let Some(value) = read_attribute(&from, "user.pairpane.origin", 4096) else {
        // A volume that keeps no user attributes: nothing to try.
        return;
    };
    assert_eq!(value, b"0083;5f3c;Safari;");

    write_attribute(&to, "user.pairpane.origin", &value);
    assert_eq!(read_attribute(&to, "user.pairpane.origin", 4096), Some(value));
    assert_eq!(read_attribute(&to, "user.pairpane.other", 4096), None);
    assert_eq!(
        read_attribute(&to, "user.pairpane.origin", 4),
        None,
        "a marker longer than the limit is not read"
    );
}

#[cfg(windows)]
#[test]
fn the_zone_stream_of_an_archive_is_put_on_a_written_file() {
    let root = temp();
    let (archive, file) = archive_and_file(&root);
    let marker = b"[ZoneTransfer]\r\nZoneId=3\r\n";
    fs::write(root.path().join("a.zip:Zone.Identifier"), marker).expect("stream");

    let origin = Origin::of(&archive);
    origin.apply(&file);

    assert_eq!(origin.bytes(), Some(&marker[..]));
    assert_eq!(read(&root.path().join("out.txt:Zone.Identifier")), marker);
    assert_eq!(read(&file), b"data");
}

#[cfg(target_os = "macos")]
#[test]
fn the_quarantine_attribute_of_an_archive_is_put_on_a_written_file() {
    use super::origin::attribute::{read_attribute, write_attribute};

    let root = temp();
    let (archive, file) = archive_and_file(&root);
    let marker = b"0083;5f3c0000;Safari;";
    write_attribute(&archive, "com.apple.quarantine", marker);

    let origin = Origin::of(&archive);
    origin.apply(&file);

    assert_eq!(origin.bytes(), Some(&marker[..]));
    assert_eq!(
        read_attribute(&file, "com.apple.quarantine", 4096).as_deref(),
        Some(&marker[..])
    );
    assert_eq!(read(&file), b"data");
}

/// Marks `archive` the way the platform's downloads are marked, and returns the marker.
#[cfg(any(windows, target_os = "macos"))]
fn marked(archive: &std::path::Path) -> &'static [u8] {
    #[cfg(windows)]
    {
        let marker: &[u8] = b"[ZoneTransfer]\r\nZoneId=3\r\n";
        let mut name = archive.as_os_str().to_owned();
        name.push(":Zone.Identifier");
        fs::write(name, marker).expect("stream");

        marker
    }
    #[cfg(target_os = "macos")]
    {
        let marker: &[u8] = b"0083;5f3c0000;Safari;";
        super::origin::attribute::write_attribute(archive, "com.apple.quarantine", marker);

        marker
    }
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn every_file_extracted_from_a_marked_archive_gets_the_marker_and_folders_do_not_need_it() {
    use super::ArchiveJob;
    use super::extract_tests::{fixture, job, scene};
    use super::testing::Rig;
    use crate::model::Outcome;

    let root = scene("x.tar", &fixture());
    let marker = marked(&root.path().join("src/x.tar"));

    let ended = Rig::new().run(job(root.path(), "x.tar", "", &[], ArchiveJob::default()));

    assert_eq!(ended.outcome, Outcome::Done);
    for file in ["a.txt", "docs/b.txt", "docs/deep/c.txt"] {
        let written = root.path().join("dst").join(file);

        assert_eq!(Origin::of(&written).bytes(), Some(marker), "{file}");
    }
}
