//! The question a pack job asks when the name of its archive is taken (before a byte is written), and the name it takes
//! when the answer is to rename.

use std::time::{Duration, SystemTime};

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::model::{Answer, ArchiveFormat, ConflictInfo, Outcome, PackFormat, PackLevel};

use super::conflict::free_name;
use super::pack_tests::{FORMATS, ITEMS, archived, dst, expected, expected_bytes, options, pack, scene, zip};
use super::testing::{Ended, Rig, long_ago, put, read, set_times, temp};

/// A scene whose archive `name` is taken (by "old"): what the job asks, and what is in `dst` while it waits for the
/// answer, then how it ended.
fn clash(answer: Answer, format: PackFormat, name: &str) -> (TempDir, Ended, ConflictInfo, Vec<String>) {
    let root = scene();
    put(root.path(), &format!("dst/{name}"), b"old");
    let mut rig = Rig::new();

    let job = rig.start(pack(root.path(), &ITEMS, name, options(format, PackLevel::Normal)));
    let question = rig.wait_conflict(job);
    let waiting = dst(&root);
    rig.engine
        .answer(job, question.question, answer, false)
        .expect("an open question");

    (root, rig.wait_ended(job), question, waiting)
}

#[test]
fn a_taken_name_asks_before_anything_is_written_and_overwrite_replaces_it() {
    let (root, ended, question, waiting) = clash(Answer::Overwrite, PackFormat::Zip, "out.zip");

    assert_eq!(waiting, ["out.zip"], "no temporary file yet");
    assert_eq!(question.name, "out.zip");
    assert_eq!(question.suggested, "out (2).zip");
    assert!(!question.comparable);
    assert_eq!(question.target.size, 3);
    assert_eq!(question.source.size, expected_bytes());
    assert!(!question.source.dir && question.source.modified.is_some());
    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(archived(&root, "out.zip", ArchiveFormat::Zip), expected());
    assert_eq!(dst(&root), ["out.zip"]);
}

#[test]
fn skip_keeps_the_taken_name_and_counts_it() {
    let (root, ended, _, _) = clash(Answer::Skip, PackFormat::Zip, "out.zip");

    assert_eq!((ended.outcome, ended.skipped), (Outcome::Done, 1));
    assert_eq!(read(&root.path().join("dst/out.zip")), b"old");
    assert_eq!(dst(&root), ["out.zip"]);
}

#[test]
fn rename_writes_the_archive_under_a_free_name_that_keeps_the_extensions_together() {
    for (format, kind, name) in FORMATS {
        let (root, ended, _, _) = clash(Answer::Rename, format, name);
        let renamed = name.replacen("out", "out (2)", 1);

        assert_eq!(ended.outcome, Outcome::Done, "{name}");
        assert_eq!(read(&root.path().join("dst").join(name)), b"old", "{name}");
        assert_eq!(archived(&root, &renamed, kind), expected(), "{name}");
        let mut both = vec![renamed, name.to_owned()];
        both.sort();
        assert_eq!(dst(&root), both, "{name}");
    }
}

#[test]
fn overwrite_older_replaces_a_target_older_than_the_new_archive_and_keeps_a_newer_one() {
    for (age, replaced) in [
        (long_ago(), true),
        (SystemTime::now() + Duration::from_secs(86_400), false),
    ] {
        let root = scene();
        put(root.path(), "dst/out.zip", b"old");
        set_times(&root.path().join("dst/out.zip"), age);
        let mut rig = Rig::new();

        let job = rig.start(zip(root.path()));
        rig.answer_next(job, Answer::OverwriteOlder, false);
        let ended = rig.wait_ended(job);

        assert_eq!(ended.outcome, Outcome::Done);
        assert_eq!(ended.skipped, u32::from(!replaced));
        assert_eq!(read(&root.path().join("dst/out.zip")) != b"old", replaced);
        assert_eq!(dst(&root), ["out.zip"]);
    }
}

#[test]
fn a_free_name_puts_the_number_before_the_ending_of_an_archive() {
    let folder = temp();
    let free = |name: &str| {
        free_name(&folder.path().join(name))
            .expect("a free name")
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .into_owned()
    };

    assert_eq!(free("a.tar.zst"), "a (2).tar.zst");
    assert_eq!(free("a.TAR.GZ"), "a (2).TAR.GZ");
    assert_eq!(free("a.tar.xz"), "a (2).tar.xz");
    assert_eq!(free("a.tar.bz2"), "a (2).tar.bz2");
    assert_eq!(free("a.7z"), "a (2).7z");
    assert_eq!(free("a.zip"), "a (2).zip");
    assert_eq!(free("a.tgz"), "a (2).tgz");
    assert_eq!(free("a.tar"), "a (2).tar");
    // Any other name has the rule it always had: the last extension.
    assert_eq!(free("a.b.txt"), "a.b (2).txt");
    assert_eq!(free("a.zst"), "a (2).zst");
    assert_eq!(free(".tar.zst"), ".tar (2).zst");
    assert_eq!(free("noext"), "noext (2)");

    put(folder.path(), "a (2).tar.zst", b"x");
    put(folder.path(), "a (3).tar.zst", b"x");
    assert_eq!(free("a.tar.zst"), "a (4).tar.zst");
}
