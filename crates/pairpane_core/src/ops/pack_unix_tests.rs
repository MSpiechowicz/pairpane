//! Pack jobs and the things only Unix has: links, pipes, names that are not text, and a folder swapped for a link.

use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::sync::Arc;

use pretty_assertions::assert_eq;

use crate::model::{ArchiveFormat, Outcome, PackFormat, PackLevel};

use super::hooks::Hooks;
use super::pack_tests::{ITEMS, archived, at, expected, options, pack, scene, the_paths, zip};
use super::testing::{Rig, make_fifo, put, set_times};

#[test]
fn links_and_special_files_are_skipped_and_counted_and_never_followed() {
    let root = scene();
    let src = root.path().join("src");
    let outside = root.path().join("outside");
    put(root.path(), "outside/secret.txt", b"secret");
    symlink(&outside, src.join("to-folder")).expect("link");
    symlink(outside.join("secret.txt"), src.join("docs/to-file")).expect("link");
    symlink("nowhere", src.join("docs/deep/dangling")).expect("link");
    make_fifo(&src.join("pipe"));
    // What was put into the folders changed their times.
    set_times(&src.join("docs/deep"), at(1_000_000_005));
    set_times(&src.join("docs"), at(1_000_000_006));

    let names = ["a.txt", "docs", "empty", "pipe", "to-folder", "z.bin"];
    let ended = Rig::new().run(pack(
        root.path(),
        &names,
        "out.zip",
        options(PackFormat::Zip, PackLevel::Normal),
    ));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.skipped, 4);
    assert!(ended.errors.is_empty());
    assert_eq!(archived(&root, "out.zip", ArchiveFormat::Zip), expected());
}

#[test]
fn a_name_that_is_not_text_or_cannot_be_read_back_fails_its_item_and_the_job_goes_on() {
    let root = scene();
    let docs = root.path().join("src/docs");
    fs::write(docs.join(std::ffi::OsStr::from_bytes(b"bad-\xff.txt")), b"x").expect("file");
    fs::write(docs.join("back\\slash.txt"), b"x").expect("file");
    // The folders get their times back: the two files above changed the one of `docs`.
    set_times(&docs, at(1_000_000_006));

    let ended = Rig::new().run(pack(
        root.path(),
        &ITEMS,
        "out.7z",
        options(PackFormat::SevenZ, PackLevel::Normal),
    ));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(
        ended.errors.iter().map(|error| error.path.as_str()).collect::<Vec<_>>(),
        ["docs/back\\slash.txt", "docs/bad-\u{fffd}.txt"]
    );
    assert_eq!(archived(&root, "out.7z", ArchiveFormat::SevenZ), expected());
}

#[test]
fn a_folder_swapped_for_a_link_while_the_tree_is_walked_is_refused_and_nothing_is_read_through_it() {
    let root = scene();
    put(root.path(), "outside/secret.txt", b"secret");
    let (docs, aside, outside) = (
        root.path().join("src/docs"),
        root.path().join("src/docs.aside"),
        root.path().join("outside"),
    );
    let hooks = Hooks {
        before_open: Some(Arc::new(move |now| {
            if now == "docs" {
                fs::rename(&docs, &aside).expect("moved aside");
                symlink(&outside, &docs).expect("link");
            }
        })),
        ..Hooks::default()
    };

    let ended = Rig::with(hooks, |_, _| {}).run(zip(root.path()));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(
        ended.errors.iter().map(|error| error.path.as_str()).collect::<Vec<_>>(),
        ["docs"]
    );
    let seen = archived(&root, "out.zip", ArchiveFormat::Zip);
    assert_eq!(the_paths(&seen), ["a.txt", "empty", "z.bin"]);
}
