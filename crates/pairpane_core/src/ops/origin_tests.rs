//! The origin marker of an archive and its copy onto what is extracted. Where the platform has no marker (Linux) the
//! tests show that nothing changes and try the decisions that make the marker; the stream (Windows) and the attribute
//! (macOS) are tried on their own systems.

use std::fs::{self, File};
use std::path::Path;

use pretty_assertions::assert_eq;

use super::origin::{Found, Origin, quarantine_marker, windows_marker};
use super::testing::{read, temp};

/// The marker of the archive at `path`, read the way a job reads it.
fn origin_of(path: &Path) -> Origin {
    Origin::of(&File::open(path).expect("open"), path)
}

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

    let origin = origin_of(&archive);
    origin.apply(&File::open(&file).expect("open"), &file);

    assert_eq!(origin.bytes(), None);
    assert_eq!(read(&file), b"data");
}

const ZONE_3: &[u8] = b"[ZoneTransfer]\r\nZoneId=3\r\n";

#[test]
fn a_windows_archive_that_has_no_marker_gives_none_and_a_good_one_is_copied_as_it_is() {
    assert_eq!(windows_marker(Found::Absent), None);

    for good in [
        &b"[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://example.com/\r\n"[..],
        b"\xEF\xBB\xBF[ZoneTransfer]\r\nZoneId=2\r\n",
        b"[zonetransfer]\nzoneid=4\n",
    ] {
        assert_eq!(windows_marker(Found::Marker(good.to_vec())).as_deref(), Some(good));
    }
}

#[test]
fn a_windows_marker_that_is_too_long_malformed_or_unreadable_is_replaced_never_dropped() {
    let mut long = b"[ZoneTransfer]\r\nZoneId=4\r\n".to_vec();
    long.resize(4096, b'x');

    // Too long: the zone read from the first bytes is kept.
    assert_eq!(
        windows_marker(Found::TooLong(long)).as_deref(),
        Some(&b"[ZoneTransfer]\r\nZoneId=4\r\n"[..])
    );
    // Malformed, with a zone to read, and without one.
    assert_eq!(
        windows_marker(Found::Marker(b"junk\r\nZoneId = 1\r\n".to_vec())).as_deref(),
        Some(&b"[ZoneTransfer]\r\nZoneId=1\r\n"[..])
    );
    for bytes in [
        &b""[..],
        b"junk",
        b"ZoneId=\r\n",
        b"ZoneId=9\r\n",
        b"ZoneId=34\r\n",
        b"\xFF\xFE[\0Z\0",
    ] {
        assert_eq!(
            windows_marker(Found::Marker(bytes.to_vec())).as_deref(),
            Some(ZONE_3),
            "{bytes:?}"
        );
        assert_eq!(
            windows_marker(Found::TooLong(bytes.to_vec())).as_deref(),
            Some(ZONE_3),
            "{bytes:?}"
        );
    }
    // Unreadable.
    assert_eq!(windows_marker(Found::Unreadable).as_deref(), Some(ZONE_3));
}

#[test]
fn a_quarantine_value_that_has_its_form_is_copied_and_any_other_one_is_replaced_never_dropped() {
    assert_eq!(quarantine_marker(Found::Absent, 0x5f3c_0000), None);

    for good in [&b"0083;5f3c0000;Safari;"[..], b"0181;5f3c0000;Chrome;0A1B-2C3D"] {
        assert_eq!(
            quarantine_marker(Found::Marker(good.to_vec()), 1).as_deref(),
            Some(good)
        );
    }

    let minimal = Some(&b"0081;5f3c0000;Pairpane;"[..]);
    for found in [
        Found::TooLong(b"0083;5f3c0000;Safari;".to_vec()),
        Found::TooLong(Vec::new()),
        Found::Marker(Vec::new()),
        Found::Marker(b"garbage".to_vec()),
        Found::Marker(b"0083;;Safari;".to_vec()),
        Found::Marker(b"0083;5f3c0000".to_vec()),
        Found::Marker(b"zzzz;5f3c0000;Safari;".to_vec()),
        Found::Unreadable,
    ] {
        assert_eq!(quarantine_marker(found, 0x5f3c_0000).as_deref(), minimal);
    }
}

/// The attribute calls the macOS marker is made of, tried on a `user.` attribute where Linux has them.
#[cfg(target_os = "linux")]
#[test]
fn an_extended_attribute_of_an_open_file_is_read_whole_too_long_or_missing_and_copied_to_another() {
    use super::origin::attribute::{read_from, write_to};

    let root = temp();
    let from = root.path().join("from");
    let to = root.path().join("to");
    fs::write(&from, b"a").expect("file");
    fs::write(&to, b"b").expect("file");
    let (from, to) = (File::open(&from).expect("open"), File::open(&to).expect("open"));
    let name = "user.pairpane.origin";

    assert_eq!(read_from(&from, name, 4096), Found::Absent);

    write_to(&from, name, b"0083;5f3c;Safari;");
    let Found::Marker(value) = read_from(&from, name, 4096) else {
        // A volume that keeps no user attributes: nothing to try.
        return;
    };
    assert_eq!(value, b"0083;5f3c;Safari;");

    write_to(&to, name, &value);
    assert_eq!(read_from(&to, name, 4096), Found::Marker(value));
    assert_eq!(read_from(&to, "user.pairpane.other", 4096), Found::Absent);
    assert!(
        matches!(read_from(&to, name, 4), Found::TooLong(_)),
        "a marker longer than the limit is said to be"
    );
}

/// Marks `archive` the way the platform's downloads are marked with `marker`.
#[cfg(any(windows, target_os = "macos"))]
fn mark(archive: &Path, marker: &[u8]) {
    #[cfg(windows)]
    {
        let mut name = archive.as_os_str().to_owned();
        name.push(":Zone.Identifier");
        fs::write(name, marker).expect("stream");
    }
    #[cfg(target_os = "macos")]
    super::origin::attribute::write_attribute(archive, "com.apple.quarantine", marker);
}

/// The marker on `file`, if any.
#[cfg(any(windows, target_os = "macos"))]
fn marker_of(file: &Path) -> Option<Vec<u8>> {
    #[cfg(windows)]
    {
        let mut name = file.as_os_str().to_owned();
        name.push(":Zone.Identifier");

        fs::read(name).ok()
    }
    #[cfg(target_os = "macos")]
    super::origin::attribute::read_attribute(file, "com.apple.quarantine")
}

/// A marker the platform's decision copies as it is.
#[cfg(any(windows, target_os = "macos"))]
const GOOD: &[u8] = if cfg!(windows) {
    b"[ZoneTransfer]\r\nZoneId=3\r\n"
} else {
    b"0083;5f3c0000;Safari;"
};

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn the_marker_of_an_archive_is_read_from_it_and_put_on_a_written_file() {
    let root = temp();
    let (archive, file) = archive_and_file(&root);
    mark(&archive, GOOD);

    let origin = origin_of(&archive);
    origin.apply(&File::options().write(true).open(&file).expect("open"), &file);

    assert_eq!(origin.bytes(), Some(GOOD));
    assert_eq!(marker_of(&file).as_deref(), Some(GOOD));
    assert_eq!(read(&file), b"data");
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn every_file_extracted_from_a_marked_archive_gets_the_marker_and_keeps_the_time_of_its_entry() {
    use super::ArchiveJob;
    use super::extract_tests::{fixture, job, moment, scene};
    use super::testing::{Rig, modified};
    use crate::model::Outcome;

    let root = scene("x.tar", &fixture());
    mark(&root.path().join("src/x.tar"), GOOD);

    let ended = Rig::new().run(job(root.path(), "x.tar", "", &[], ArchiveJob::default()));

    assert_eq!(ended.outcome, Outcome::Done);
    for file in ["a.txt", "docs/b.txt", "docs/deep/c.txt"] {
        let written = root.path().join("dst").join(file);

        assert_eq!(marker_of(&written).as_deref(), Some(GOOD), "{file}");
        assert_eq!(modified(&written), moment(), "{file}");
    }
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn a_read_only_entry_still_gets_the_marker_and_an_oversized_marker_is_replaced_by_a_minimal_one() {
    use super::ArchiveJob;
    use super::extract_tests::{add, job, scene, tar_by};
    use super::testing::Rig;
    use crate::model::Outcome;

    let root = scene("x.tar", &tar_by(|builder| add(builder, "ro.txt", 0o444, b"data")));
    let archive = root.path().join("src/x.tar");
    let mut big = if cfg!(windows) {
        b"[ZoneTransfer]\r\nZoneId=4\r\n".to_vec()
    } else {
        GOOD.to_vec()
    };
    big.resize(8192, b' ');
    mark(&archive, &big);
    if marker_of(&archive).is_none() {
        // A volume that does not take a marker that long: nothing to try.
        return;
    }

    let ended = Rig::new().run(job(root.path(), "x.tar", "", &[], ArchiveJob::default()));
    let marker = marker_of(&root.path().join("dst/ro.txt")).expect("a marker");

    assert_eq!(ended.outcome, Outcome::Done);
    if cfg!(windows) {
        // The zone of the first bytes is kept.
        assert_eq!(marker, b"[ZoneTransfer]\r\nZoneId=4\r\n");
    } else {
        // The seconds are the time of the job.
        assert!(
            marker.starts_with(b"0081;") && marker.ends_with(b";Pairpane;"),
            "{marker:?}"
        );
    }
}
