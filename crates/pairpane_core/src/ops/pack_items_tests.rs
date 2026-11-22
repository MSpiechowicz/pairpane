//! Pack jobs whose items are paths below the folder (what a list of search results gives): each is stored under its path,
//! the folders on the way are not entries unless they are items, and an item that cannot be found fails alone.

use std::fs;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::model::{ArchiveFormat, Outcome, PackFormat, PackLevel};

use super::pack_tests::{FORMATS, archived, dst, options, pack, the_paths};
use super::testing::{Ended, Rig, put, temp};

/// `src` with the files `a/b.txt`, `c/d/e.txt`, `c/d/other.txt`, `c/f.txt` and `z.txt`, and the empty `dst`.
pub(super) fn results() -> TempDir {
    let root = temp();

    for (path, bytes) in [
        ("a/b.txt", "bee"),
        ("c/d/e.txt", "eee"),
        ("c/d/other.txt", "other"),
        ("c/f.txt", "eff"),
        ("z.txt", "zed"),
    ] {
        put(root.path(), &format!("src/{path}"), bytes.as_bytes());
    }
    fs::create_dir_all(root.path().join("dst")).expect("folder");

    root
}

/// Packs `names` of the scene into `out.zip` and gives how the job ended and the paths of the archive.
pub(super) fn packed(root: &TempDir, names: &[&str]) -> (Ended, Vec<String>) {
    let ended = Rig::new().run(pack(
        root.path(),
        names,
        "out.zip",
        options(PackFormat::Zip, PackLevel::Normal),
    ));
    let seen = archived(root, "out.zip", ArchiveFormat::Zip);

    (ended, the_paths(&seen).into_iter().map(str::to_owned).collect())
}

#[test]
fn paths_below_the_folder_are_stored_under_those_paths_whatever_the_order_and_the_format() {
    for (format, kind, name) in FORMATS {
        let root = results();

        let ended = Rig::new().run(pack(
            root.path(),
            &["z.txt", "c/d/e.txt", "a/b.txt"],
            name,
            options(format, PackLevel::Normal),
        ));

        assert_eq!(ended.outcome, Outcome::Done, "{format:?}");
        assert_eq!((ended.skipped, ended.errors.len()), (0, 0), "{format:?}");
        let seen = archived(&root, name, kind);
        let content = |seen: &super::pack_tests::Seen| (seen.0.clone(), seen.4.clone());
        assert_eq!(
            seen.iter().map(content).collect::<Vec<_>>(),
            [
                ("a/b.txt".to_owned(), b"bee".to_vec()),
                ("c/d/e.txt".to_owned(), b"eee".to_vec()),
                ("z.txt".to_owned(), b"zed".to_vec()),
            ],
            "{format:?}: the folders on the way are not entries"
        );
        assert_eq!(dst(&root), [name], "no temporary file is left");
    }
}

#[test]
fn a_folder_item_below_the_folder_brings_what_is_in_it_under_its_path() {
    let root = results();

    let (ended, paths) = packed(&root, &["c/d", "z.txt"]);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(paths, ["c/d", "c/d/e.txt", "c/d/other.txt", "z.txt"]);
}

#[test]
fn an_item_that_is_gone_or_has_no_folder_on_its_way_fails_alone_and_the_job_goes_on() {
    let root = results();

    let (ended, paths) = packed(&root, &["z.txt/x", "nope/x.txt", "a/nope.txt", "a/b.txt"]);

    assert_eq!(ended.outcome, Outcome::Done);
    let failed = ended.errors.iter().map(|error| error.path.as_str()).collect::<Vec<_>>();
    assert_eq!(failed, ["a/nope.txt", "nope/x.txt", "z.txt/x"]);
    assert_eq!(paths, ["a/b.txt"]);
}

#[test]
fn a_job_whose_every_item_is_gone_has_nothing_to_pack() {
    let root = results();

    let request = pack(
        root.path(),
        &["nope/x.txt"],
        "out.zip",
        options(PackFormat::Zip, PackLevel::Normal),
    );
    let ended = Rig::new().run(request);

    assert!(matches!(ended.outcome, Outcome::Failed { .. }), "{:?}", ended.outcome);
    assert_eq!(ended.errors.len(), 1);
    assert_eq!(dst(&root), Vec::<String>::new());
}
