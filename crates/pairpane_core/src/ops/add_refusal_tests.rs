//! Add jobs on archives that a rewrite would damage: what it could not carry over is refused, the archive is left as it
//! was and no temporary file stays.

use std::fs;

use crate::archive::rewrite::testing::{TARS, archive};
use crate::archive::write::testing::Item;
use crate::model::{ArchiveFormat, IpcErrorCode, Outcome};

use super::add_tests::{KINDS, add, knows, scene_with};
use super::testing::{Rig, has_no_partial_files, read};

/// The archive `name` of `root`'s `dst` replaced by `bytes`.
fn replace(root: &tempfile::TempDir, name: &str, bytes: &[u8]) {
    fs::write(root.path().join("dst").join(name), bytes).expect("an archive");
}

/// The text a job `name` of the scene fails with.
fn failure_of(root: &tempfile::TempDir, name: &str) -> String {
    let ended = Rig::new().run(add(root.path(), &["new.txt"], name, "", knows(None)));

    let Outcome::Failed { error } = ended.outcome else {
        panic!("{name}: the job must fail: {:?}", ended.outcome);
    };
    assert_ne!(error.code, IpcErrorCode::PasswordRequired, "{name}");

    error.message
}

#[test]
fn a_tar_with_something_after_its_end_is_refused_in_every_codec_and_left_as_it_was() {
    for (name, format, _, _) in KINDS.iter().filter(|kind| TARS.contains(&kind.1)) {
        let root = scene_with(name, *format, None, false);

        let mut bytes = read(&root.path().join("dst").join(name));
        bytes.extend(archive(
            *format,
            &[Item::file("second.txt", 0o644, b"a second tar")],
            None,
            false,
        ));
        replace(&root, name, &bytes);

        assert_eq!(
            failure_of(&root, name),
            "the archive has data after its end, so it can't be changed",
            "{format:?}"
        );
        assert_eq!(read(&root.path().join("dst").join(name)), bytes, "{format:?}");
        assert!(has_no_partial_files(&root.path().join("dst")), "{format:?}");
    }
}

#[test]
fn a_zip_with_a_program_before_its_entries_is_refused_and_left_as_it_was() {
    let root = scene_with("a.zip", ArchiveFormat::Zip, None, false);
    // Starts like a zip, so it is opened as one (a program that unpacks itself starts otherwise and is not opened at all).
    let mut bytes = b"PK\x03\x04 a stub that is not part of the archive".to_vec();
    bytes.extend(read(&root.path().join("dst/a.zip")));
    replace(&root, "a.zip", &bytes);

    assert_eq!(
        failure_of(&root, "a.zip"),
        "this archive has data before its entries, so it can't be changed"
    );
    assert_eq!(read(&root.path().join("dst/a.zip")), bytes);
    assert!(has_no_partial_files(&root.path().join("dst")));
}
