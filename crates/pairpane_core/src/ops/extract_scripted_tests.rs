//! Extractions whose source misbehaves (more data than declared, an entry that cannot be read, a wrong password), an
//! archive that is not what it was, and the requests that are refused. The source is scripted through a test hook; the
//! archive file is a real one, so the job opens, fingerprints and indexes it as it always does.

use std::fs;
use std::io::{self, Cursor, Read};
use std::sync::Arc;

use pretty_assertions::assert_eq;

use crate::archive::testing::MOMENT;
use crate::archive::{
    ArchiveIndex, Builder, EntrySource, EntryType, Fingerprint, Flow, IndexEntry, PasswordFault, Place, Record, Visit,
    password_error,
};
use crate::model::{ArchiveFormat, IpcErrorCode, OpKind, Outcome};

use super::extract_tests::{dst, fixture, job, scene};
use super::testing::{Ended, Rig, has_no_partial_files, temp, tree};
use super::{ArchiveJob, MAX_NAMES, Refused, Request, Task};

/// What a scripted source gives for one entry.
#[derive(Clone)]
enum Fate {
    Bytes(&'static [u8]),
    /// More bytes than the entry declares.
    Flood,
    /// Reading fails with this error.
    Breaks(io::ErrorKind, &'static str),
    /// The source itself fails with this.
    Password(PasswordFault),
}

struct Scripted {
    fates: Vec<Fate>,
    entries: Vec<IndexEntry>,
}

impl EntrySource for Scripted {
    fn extract(&mut self, wanted: &[u32], visit: &mut Visit<'_>) -> io::Result<()> {
        for ordinal in wanted {
            let reader: Box<dyn Read> = match &self.fates[*ordinal as usize] {
                Fate::Bytes(bytes) => Box::new(Cursor::new(*bytes)),
                Fate::Flood => Box::new(io::repeat(7)),
                Fate::Breaks(kind, text) => Box::new(Breaks(*kind, text)),
                Fate::Password(fault) => return Err(password_error(*fault)),
            };

            if visit(&self.entries[*ordinal as usize], &mut { reader })? == Flow::Stop {
                break;
            }
        }

        Ok(())
    }
}

struct Breaks(io::ErrorKind, &'static str);

impl Read for Breaks {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(self.0, self.1))
    }
}

/// A rig whose extractions read from the scripted `fates`.
fn scripted(fates: Vec<Fate>) -> Rig {
    Rig::hooked(|hooks| {
        hooks.entry_source = Some(Arc::new(move |index: &ArchiveIndex| -> Box<dyn EntrySource> {
            Box::new(Scripted {
                fates: fates.clone(),
                entries: index.entries().to_vec(),
            })
        }));
    })
}

/// What a listing of the archive `x.tar` in `root/src` made: files `(name, declared size, encrypted)`, and the state of
/// the file.
fn listed(root: &std::path::Path, files: &[(&str, u64, bool)]) -> ArchiveJob {
    let mut builder = Builder::new(ArchiveFormat::Tar);

    for (name, size, encrypted) in files {
        builder.push(Record {
            name: (*name).to_owned(),
            kind: EntryType::File,
            size: *size,
            modified: Some(MOMENT),
            mode: Some(0o644),
            link: None,
            encrypted: *encrypted,
            place: Place::Walk,
        });
    }

    let path = root.join("src/x.tar");

    ArchiveJob {
        index: Some(Arc::new(builder.finish(false))),
        fingerprint: Some(Fingerprint::of(&path, &fs::metadata(&path).expect("the archive"))),
        password: None,
    }
}

fn failure(ended: &Ended) -> (IpcErrorCode, String) {
    match &ended.outcome {
        Outcome::Failed { error } => (error.code, error.message.clone()),
        other => panic!("the job fails: {other:?}"),
    }
}

fn error_of(ended: &Ended, at: usize) -> (&str, &str) {
    let error = &ended.errors[at];

    (&error.path, &error.error.message)
}

#[test]
fn an_entry_with_more_data_than_it_declares_fails_and_leaves_no_partial_file_and_the_rest_goes_on() {
    let root = scene("x.tar", &fixture());
    let source = listed(
        root.path(),
        &[
            ("big.bin", 4, false),
            ("ok.txt", 2, false),
            ("m.bin", 3, false),
            ("c.bin", 3, false),
        ],
    );
    let mut rig = scripted(vec![
        Fate::Flood,
        Fate::Bytes(b"ok"),
        Fate::Breaks(io::ErrorKind::Unsupported, "this compression method is not handled"),
        Fate::Breaks(io::ErrorKind::InvalidData, "the checksum is wrong"),
    ]);

    let ended = rig.run(job(root.path(), "x.tar", "", &[], source));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(
        error_of(&ended, 0),
        ("big.bin", "an entry holds more data than it declares")
    );
    assert_eq!(error_of(&ended, 1), ("m.bin", "this compression method is not handled"));
    assert_eq!(error_of(&ended, 2), ("c.bin", "the checksum is wrong"));
    assert_eq!(dst(&root), ["ok.txt"]);
    assert!(has_no_partial_files(root.path()));
}

#[test]
fn a_password_that_is_missing_or_wrong_ends_the_job_as_a_password_error() {
    let root = scene("x.tar", &fixture());
    let mut rig = scripted(vec![Fate::Bytes(b"abc")]);

    let source = listed(root.path(), &[("secret.txt", 3, true)]);
    let ended = rig.run(job(root.path(), "x.tar", "", &[], source));
    assert_eq!(
        failure(&ended),
        (IpcErrorCode::PasswordRequired, "needs a password".to_owned())
    );
    assert!(!root.path().join("dst").exists(), "nothing was written");

    let mut rig = scripted(vec![Fate::Password(PasswordFault::Wrong)]);
    let mut source = listed(root.path(), &[("secret.txt", 3, true)]);
    source.password = Some(Arc::new(crate::archive::Password::new("guess".to_owned())));
    let ended = rig.run(job(root.path(), "x.tar", "", &[], source));
    assert_eq!(
        failure(&ended),
        (IpcErrorCode::PasswordRequired, "the password is wrong".to_owned())
    );
}

#[test]
fn a_password_that_the_archive_accepts_lets_an_encrypted_entry_through() {
    let root = scene("x.tar", &fixture());
    let mut source = listed(root.path(), &[("secret.txt", 3, true)]);
    source.password = Some(Arc::new(crate::archive::Password::new("right".to_owned())));

    let ended = scripted(vec![Fate::Bytes(b"abc")]).run(job(root.path(), "x.tar", "", &[], source));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(dst(&root), ["secret.txt"]);
}

#[test]
fn an_archive_that_changed_since_it_was_listed_is_refused() {
    let root = scene("x.tar", &fixture());
    let source = listed(root.path(), &[("a.txt", 5, false)]);
    let unlisted = ArchiveJob {
        fingerprint: None,
        ..source.clone()
    };
    fs::write(root.path().join("src/x.tar"), [fixture(), vec![0; 512]].concat()).expect("grown");
    let mut rig = Rig::new();

    for stale in [source, unlisted] {
        let ended = rig.run(job(root.path(), "x.tar", "", &[], stale));

        assert_eq!(failure(&ended).1, "the archive changed; open it again");
    }
    assert!(!root.path().join("dst").exists());
}

#[test]
fn what_is_not_an_archive_or_not_in_it_fails_the_job_with_a_clear_text() {
    let root = scene("x.tar", &fixture());
    fs::write(root.path().join("src/plain.tar"), b"not a tar at all").expect("file");
    let mut rig = Rig::new();
    let mut run = |name: &str, inner: &str, entries: &[&str]| {
        failure(&rig.run(job(root.path(), name, inner, entries, ArchiveJob::default()))).1
    };

    assert_eq!(run("plain.tar", "", &[]), "this is not an archive that can be opened");
    assert_eq!(run("x.tar", "nope", &[]), "the folder or item is not in the archive");
    assert_eq!(run("x.tar", "", &["zzz"]), "the folder or item is not in the archive");
    assert_eq!(run("gone.tar", "", &[]), "it no longer exists");
    assert_eq!(
        tree(root.path()).iter().filter(|path| path.starts_with("dst")).count(),
        0
    );
}

#[cfg(unix)]
#[test]
fn an_archive_that_is_a_link_is_refused() {
    let root = scene("x.tar", &fixture());
    std::os::unix::fs::symlink(root.path().join("src/x.tar"), root.path().join("src/link.tar")).expect("link");

    let ended = Rig::new().run(job(root.path(), "link.tar", "", &[], ArchiveJob::default()));

    assert_eq!(failure(&ended).1, "the archive is not a regular file");
}

#[test]
fn a_disk_that_would_be_left_without_its_reserve_stops_the_job_before_a_byte_is_written() {
    let root = scene("x.tar", &fixture());
    let mut rig = Rig::hooked(|hooks| hooks.free_reserve = Some(u64::MAX));

    let ended = rig.run(job(root.path(), "x.tar", "", &[], ArchiveJob::default()));

    assert_eq!(
        failure(&ended),
        (
            IpcErrorCode::Io,
            "there is not enough free space left on the disk".to_owned()
        )
    );
    assert_eq!(
        tree(root.path()),
        ["src/", "src/x.tar"],
        "no file, no partial file, no folder"
    );
}

/// The result of submitting an extraction of `entries` from the folder `inner`, with `names` as the archive.
fn submitted(inner: &str, entries: Vec<String>, names: &[&str], kind: OpKind) -> Result<(), Refused> {
    let root = temp();
    let mut request: Request = job(root.path(), "x.tar", inner, &[], ArchiveJob::default());
    request.names = names.iter().map(|name| (*name).to_owned()).collect();
    request.kind = kind;
    request.task = Task::Extract {
        inner: inner.to_owned(),
        entries,
        source: ArchiveJob::default(),
    };
    let mut rig = Rig::new();
    let accepted = rig.engine.submit(request);

    // An accepted job fails by itself later, on a folder that has no archive: wait for it.
    if let Ok(id) = accepted {
        rig.wait_ended(id);
    }

    accepted.map(|_| ())
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn a_request_needs_one_archive_plain_unique_entries_and_a_normal_folder() {
    let ok = |inner: &str, entries: &[&str]| submitted(inner, names(entries), &["x.tar"], OpKind::Extract);

    assert_eq!(ok("", &[]), Ok(()));
    assert_eq!(ok("a/b", &["one", "two"]), Ok(()));

    assert_eq!(ok("", &["a", "a"]), Err(Refused::InvalidNames), "twice");
    assert_eq!(ok("", &["a/b"]), Err(Refused::InvalidNames), "not a plain name");
    assert_eq!(ok("", &[".."]), Err(Refused::InvalidNames));
    assert_eq!(ok("", &[&"x".repeat(300)]), Err(Refused::NamesTooLarge));
    for inner in ["../x", "a/../b", "a//b", "/a", "a/", "a\\b", "C:x", "./a"] {
        assert_eq!(ok(inner, &[]), Err(Refused::InvalidTask), "{inner}");
    }

    let many: Vec<String> = (0..=MAX_NAMES).map(|number| number.to_string()).collect();
    assert_eq!(
        submitted("", many, &["x.tar"], OpKind::Extract),
        Err(Refused::InvalidNames)
    );
    assert_eq!(
        submitted("", Vec::new(), &["x.tar", "y.tar"], OpKind::Extract),
        Err(Refused::InvalidTask)
    );
    assert_eq!(
        submitted("", Vec::new(), &[], OpKind::Extract),
        Err(Refused::InvalidNames)
    );
    assert_eq!(
        submitted("", Vec::new(), &["x.tar"], OpKind::Copy),
        Err(Refused::InvalidTask)
    );
}
