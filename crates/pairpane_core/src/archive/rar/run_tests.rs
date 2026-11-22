//! Listing and unpacking through `unrar`. The program is the real one where it is installed (the tests skip themselves
//! where it is not), on archives written by `testing.rs`; the fixed `true` and `false` stand in for a program that
//! unpacks nothing.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::run::{extract_args, list_args};
use super::testing::{Item, MOMENT, index_of, rar_of, rar_with};
use super::{RarSource, Unrar, headers_encrypted, is_missing, protected, records_with};
use crate::archive::entries::{EntrySource, EntryType, Flow, Place, Record};
use crate::archive::error::{password_fault, unreadable};
use crate::archive::{ArchiveIndex, Builder, Input};
use crate::model::ArchiveFormat;
use crate::view::is_refusal;

/// An entry the visitor saw: its path, and its content or the kind of the error its reader failed with.
type Seen = (String, Result<Vec<u8>, io::ErrorKind>);

static IDLE: AtomicBool = AtomicBool::new(false);

/// An archive on disk, in a folder that is also where unpacking is staged, so the folder shows what is left behind.
struct Rig {
    folder: TempDir,
    archive: PathBuf,
    unrar: Unrar,
}

fn rig_with(program: Option<Unrar>, bytes: &[u8]) -> Option<Rig> {
    let unrar = match program {
        Some(unrar) => unrar,
        None => Unrar::locate().ok()?,
    };
    let folder = TempDir::new().expect("folder");
    let archive = folder.path().join("test.rar");
    fs::write(&archive, bytes).expect("archive");

    Some(Rig { folder, archive, unrar })
}

/// A rig with the real `unrar`; `None` where it is not installed.
fn rig(items: &[Item<'_>]) -> Option<Rig> {
    rig_with(None, &rar_of(items))
}

impl Rig {
    fn list(&self) -> io::Result<Vec<Record>> {
        let file = File::open(&self.archive).expect("open");
        let input = Input {
            file: &file,
            path: Some(&self.archive),
        };
        let mut records = Vec::new();

        records_with(&self.unrar, &input, &IDLE, &mut |record| {
            records.push(record);

            Flow::Continue
        })?;

        Ok(records)
    }

    fn index(&self) -> ArchiveIndex {
        let mut builder = Builder::new(ArchiveFormat::Rar);
        for record in self.list().expect("a listing") {
            builder.push(record);
        }

        builder.finish(false)
    }

    /// Unpacks `wanted` (ordinals) of `index`, collecting `(path, content or the error's kind)` of each entry visited.
    fn unpack(&self, index: &ArchiveIndex, wanted: &[u32]) -> io::Result<Vec<Seen>> {
        let mut source =
            RarSource::new(index, &self.archive, self.unrar.clone(), &IDLE).staging_in(self.folder.path().to_owned());
        let mut seen = Vec::new();

        source.extract(wanted, &mut |entry, reader| {
            let mut data = Vec::new();
            let read = reader
                .read_to_end(&mut data)
                .map(|_| data)
                .map_err(|error| error.kind());
            seen.push((entry.path.clone(), read));

            Ok(Flow::Continue)
        })?;

        Ok(seen)
    }

    /// The names in the folder: the archive, and nothing a failed or finished unpacking leaves.
    fn leftovers(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.folder.path())
            .expect("folder")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();

        names
    }
}

fn ordinals(index: &ArchiveIndex, dir: &str, names: &[&str]) -> Vec<u32> {
    let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();

    index.select(dir, &names).expect("selectable").ordinals()
}

#[test]
fn the_arguments_end_the_switches_before_any_path_and_the_destination_comes_last() {
    let archive = Path::new("/data/-x.rar");
    // Relative to the working folder, which is the staging folder: never an absolute path `unrar` could make again.
    let destination = format!(".{}", std::path::MAIN_SEPARATOR);

    let list = list_args(archive);
    assert_eq!(list.last().expect("last"), "/data/-x.rar");
    assert_eq!(list[list.len() - 2], "--");

    let all = extract_args(archive, None);
    assert_eq!(all[all.len() - 3..], ["--", "/data/-x.rar", destination.as_str()]);
    assert!(all.iter().any(|arg| arg == "-ol-"), "links are skipped by unrar itself");

    let named = extract_args(archive, Some(&["-r", "a/@b"]));
    assert_eq!(
        named[named.len() - 5..],
        ["--", "/data/-x.rar", "-r", "a/@b", destination.as_str()]
    );
}

#[test]
fn the_archive_and_the_staging_folder_must_be_plain_absolute_paths() {
    let unrar = Unrar::at(PathBuf::from("/usr/bin/true"));

    for archive in ["relative.rar", "/data/*.rar", "/data/a?.rar"] {
        let error = unrar.list(Path::new(archive), &IDLE).expect_err(archive);

        assert_eq!(error.kind(), io::ErrorKind::Unsupported, "{archive}");
        assert!(is_refusal(&error));
    }
}

#[test]
fn a_program_that_is_not_there_is_the_missing_error() {
    let program = std::env::temp_dir().join("nonexistent-pairpane").join("unrar");
    let archive = std::env::temp_dir().join("a.rar");
    let error = Unrar::at(program).list(&archive, &IDLE).expect_err("no program");

    assert!(is_missing(&error));
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert_eq!(error.to_string(), "Install unrar to open RAR archives");
    assert!(!is_missing(&unreadable()));
}

#[test]
fn a_raised_flag_starts_nothing() {
    let cancel = AtomicBool::new(true);

    let error = Unrar::at(PathBuf::from("/nonexistent/pairpane/unrar"))
        .list(Path::new("/data/a.rar"), &cancel)
        .expect_err("cancelled");

    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert!(cancel.load(Ordering::Relaxed));
}

#[test]
fn encrypted_headers_are_told_from_the_first_header_alone() {
    let plain = rar_of(&[Item::File("a.txt", b"a")]);
    assert!(!headers_encrypted(&plain).expect("read"));

    // RAR 5: the archive encryption header (type 4) follows the signature, then the encrypted headers.
    let mut protected_5 = b"Rar!\x1A\x07\x01\x00".to_vec();
    protected_5.extend_from_slice(&[0, 0, 0, 0, 0x0D, 4, 0, 0]);
    assert!(headers_encrypted(&protected_5).expect("read"));

    // RAR 4: the main header (type 0x73) has the password flag.
    let mut protected_4 = b"Rar!\x1A\x07\x00".to_vec();
    protected_4.extend_from_slice(&[0, 0, 0x73, 0x80, 0x00, 13, 0]);
    assert!(headers_encrypted(&protected_4).expect("read"));
    protected_4[4] = 0;
    protected_4[5] = 0;
    assert!(!headers_encrypted(&protected_4).expect("read"));

    assert!(!headers_encrypted(&b"PK\x03\x04".to_vec()).expect("read"));
}

#[test]
fn an_archive_with_encrypted_headers_is_refused_before_unrar_is_started() {
    let mut bytes = b"Rar!\x1A\x07\x01\x00".to_vec();
    bytes.extend_from_slice(&[0, 0, 0, 0, 0x0D, 4, 0, 0]);
    let rig = rig_with(Some(Unrar::at(PathBuf::from("/nonexistent/pairpane/unrar"))), &bytes).expect("rig");

    let error = rig.list().expect_err("protected");

    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    assert_eq!(error.to_string(), "password-protected RAR archives are not supported");
    assert_eq!(password_fault(&error), None, "not a password to ask for");
}

#[test]
fn an_entry_with_encrypted_content_is_refused_when_it_is_unpacked_and_folders_are_not() {
    let mut builder = Builder::new(ArchiveFormat::Rar);
    for (name, kind, encrypted) in [("d", EntryType::Dir, false), ("d/secret.txt", EntryType::File, true)] {
        builder.push(Record {
            name: name.to_owned(),
            kind,
            size: 5,
            modified: None,
            mode: None,
            link: None,
            encrypted,
            place: Place::Walk,
        });
    }
    let index = builder.finish(false);
    // Nothing is started for it: the program is not there.
    let rig = rig_with(Some(Unrar::at(PathBuf::from("/nonexistent/pairpane/unrar"))), b"").expect("rig");

    assert_eq!(rig.unpack(&index, &[0]).expect("a folder").len(), 1);

    let error = rig.unpack(&index, &[0, 1]).expect_err("protected");
    assert_eq!(error.to_string(), protected().to_string());
    assert_eq!(rig.leftovers(), ["test.rar"]);
}

#[test]
#[cfg(unix)]
fn a_program_that_unpacks_nothing_fails_each_file_and_leaves_nothing() {
    for program in ["/usr/bin/true", "/usr/bin/false"] {
        let Some(rig) = rig_with(Some(Unrar::at(PathBuf::from(program))), &rar_of(&[])) else {
            continue;
        };
        let mut builder = Builder::new(ArchiveFormat::Rar);
        for (name, kind) in [("d", EntryType::Dir), ("d/a.txt", EntryType::File)] {
            builder.push(Record {
                name: name.to_owned(),
                kind,
                size: 1,
                modified: None,
                mode: None,
                link: None,
                encrypted: false,
                place: Place::Walk,
            });
        }
        let index = builder.finish(false);

        let seen = rig.unpack(&index, &[0, 1]).expect("no error of the whole");

        assert_eq!(
            seen,
            [
                ("d".to_owned(), Ok(vec![])),
                ("d/a.txt".to_owned(), Err(io::ErrorKind::InvalidData))
            ],
            "{program}"
        );
        assert_eq!(rig.leftovers(), ["test.rar"], "{program}");
    }
}

#[test]
fn unpacking_what_does_not_fit_on_the_disk_is_refused_before_it_starts() {
    let index = index_of(&[("huge.bin", 1 << 62)]);
    let rig = rig_with(Some(Unrar::at(PathBuf::from("/usr/bin/true"))), b"").expect("rig");

    let error = rig.unpack(&index, &[0]).expect_err("too big");

    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    assert!(is_refusal(&error));
    assert_eq!(rig.leftovers(), ["test.rar"]);
}

#[test]
fn the_listing_of_unrar_is_the_entries_of_the_archive() {
    let Some(rig) = rig(&[
        Item::Dir("d"),
        Item::File("d/a.txt", b"hello a\n"),
        Item::Moded("b.txt", b"bee\n", 0o4755),
        Item::Symlink("ln", "b.txt"),
        Item::Hardlink("hard", "b.txt"),
    ]) else {
        return;
    };

    let records = rig.list().expect("a listing");

    let summary: Vec<_> = records
        .iter()
        .map(|r| (r.name.as_str(), r.kind, r.size, r.mode))
        .collect();
    assert_eq!(
        summary,
        [
            ("d", EntryType::Dir, 0, Some(0o755)),
            ("d/a.txt", EntryType::File, 8, Some(0o644)),
            ("b.txt", EntryType::File, 4, Some(0o4755)),
            ("ln", EntryType::Symlink, 0, Some(0o777)),
            ("hard", EntryType::Hardlink, 0, Some(0o777)),
        ]
    );
    assert_eq!(
        records[1].modified,
        Some(i64::from(MOMENT) * 1000),
        "times are read as UTC"
    );
    assert_eq!(records[3].link.as_deref(), Some("b.txt"));
}

#[test]
fn a_file_that_is_not_an_archive_and_a_volume_of_a_set_are_refused() {
    let Some(garbage) = rig_with(None, b"this is not a RAR archive") else {
        return;
    };
    assert_eq!(
        garbage.list().expect_err("not an archive").kind(),
        io::ErrorKind::InvalidData
    );

    let volume = rig_with(None, &rar_with(&[Item::File("a.txt", b"a")], 1)).expect("rig");
    let error = volume.list().expect_err("a volume");
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    assert_eq!(error.to_string(), "multi-volume RAR archives are not supported");
}

#[test]
fn chosen_entries_are_unpacked_to_the_visitor_and_nothing_is_left() {
    let Some(rig) = rig(&[
        Item::Dir("d"),
        Item::File("d/a.txt", b"hello a\n"),
        Item::File("b.txt", b"bee\n"),
        Item::File("-r", b"dash"),
        Item::File("@list", b"at"),
    ]) else {
        return;
    };
    let index = rig.index();

    let wanted = ordinals(&index, "", &["d", "-r", "@list"]);
    let seen = rig.unpack(&index, &wanted).expect("unpacked");

    assert_eq!(
        seen,
        [
            ("d".to_owned(), Ok(vec![])),
            ("d/a.txt".to_owned(), Ok(b"hello a\n".to_vec())),
            ("-r".to_owned(), Ok(b"dash".to_vec())),
            ("@list".to_owned(), Ok(b"at".to_vec())),
        ],
        "b.txt was not asked for"
    );
    assert_eq!(rig.leftovers(), ["test.rar"]);
}

#[test]
fn a_name_with_a_wildcard_is_picked_from_everything_unpacked_and_links_and_climbs_are_never_read() {
    let Some(rig) = rig(&[
        Item::File("glob*.txt", b"g\n"),
        Item::File("globe.txt", b"not me\n"),
        Item::File("b.txt", b"bee\n"),
        Item::Symlink("ln", "b.txt"),
        Item::Symlink("esc", "../../outside"),
        Item::Hardlink("hard", "b.txt"),
        Item::File("../evil.txt", b"evil\n"),
        Item::File("new\nline.txt", b"nl\n"),
    ]) else {
        return;
    };
    let index = rig.index();

    // The whole root is chosen: the links are left out of the selection, the refused names are not in it.
    let selection = index.select("", &[]).expect("root");
    assert_eq!(selection.skipped, 2 + 1, "two symbolic links and a hard link");
    assert_eq!(selection.refused, 2);

    let seen = rig.unpack(&index, &selection.ordinals()).expect("unpacked");

    let names: Vec<&str> = seen.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["glob*.txt", "globe.txt", "b.txt"]);
    assert_eq!(seen[0].1, Ok(b"g\n".to_vec()));
    assert_eq!(seen[1].1, Ok(b"not me\n".to_vec()));
    assert_eq!(
        rig.leftovers(),
        ["test.rar"],
        "no climb reached the folder, the staging folder is gone"
    );
}

#[test]
fn a_damaged_file_fails_alone_and_the_rest_is_unpacked() {
    let mut bytes = rar_of(&[Item::File("good.txt", b"good data"), Item::File("bad.txt", b"bad data")]);
    let at = bytes.windows(8).position(|window| window == b"bad data").expect("data");
    bytes[at] ^= 1;
    let Some(rig) = rig_with(None, &bytes) else {
        return;
    };
    let index = rig.index();

    let seen = rig.unpack(&index, &[0, 1]).expect("unpacked");

    assert_eq!(seen[0], ("good.txt".to_owned(), Ok(b"good data".to_vec())));
    assert_eq!(seen[1], ("bad.txt".to_owned(), Err(io::ErrorKind::InvalidData)));
    assert_eq!(rig.leftovers(), ["test.rar"]);
}

#[test]
fn a_raised_flag_stops_the_unpacking_and_the_staging_folder_goes_with_it() {
    let Some(rig) = rig(&[Item::File("a.txt", b"a")]) else {
        return;
    };
    let index = rig.index();
    let cancel = AtomicBool::new(false);
    let mut source =
        RarSource::new(&index, &rig.archive, rig.unrar.clone(), &cancel).staging_in(rig.folder.path().to_owned());

    let error = source
        .extract(&[0], &mut |_, _| {
            cancel.store(true, Ordering::Relaxed);

            Ok(Flow::Continue)
        })
        .and_then(|()| source.extract(&[0], &mut |_, _| Ok(Flow::Continue)))
        .expect_err("cancelled");

    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(rig.leftovers(), ["test.rar"]);
}
