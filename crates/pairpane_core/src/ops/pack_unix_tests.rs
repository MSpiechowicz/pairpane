//! Pack jobs and the things only Unix has: links, pipes, names that are not text, and a folder swapped for a link.

use std::fs;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;
use std::sync::Arc;

use pretty_assertions::assert_eq;

use crate::model::{ArchiveFormat, Outcome, PackFormat, PackLevel};

use super::hooks::Hooks;
use super::pack_items_tests::{packed, results};
use super::pack_tests::{ITEMS, archived, at, expected, options, pack, scene, the_paths, zip};
use super::testing::{Rig, make_fifo, put, set_times, temp_mode_hooks};

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
    fs::write(docs.join("back\\slash.txt"), b"x").expect("file");
    // Only Linux takes any bytes in a name: APFS refuses a name that is not UTF-8, so the file cannot be made there.
    #[cfg(target_os = "linux")]
    fs::write(docs.join(std::ffi::OsStr::from_bytes(b"bad-\xff.txt")), b"x").expect("file");
    // The folders get their times back: the two files above changed the one of `docs`.
    set_times(&docs, at(1_000_000_006));

    let ended = Rig::new().run(pack(
        root.path(),
        &ITEMS,
        "out.7z",
        options(PackFormat::SevenZ, PackLevel::Normal),
    ));

    assert_eq!(ended.outcome, Outcome::Done);
    let failed = ended.errors.iter().map(|error| error.path.as_str()).collect::<Vec<_>>();
    #[cfg(target_os = "linux")]
    assert_eq!(failed, ["docs/back\\slash.txt", "docs/bad-\u{fffd}.txt"]);
    #[cfg(not(target_os = "linux"))]
    assert_eq!(failed, ["docs/back\\slash.txt"]);
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

#[test]
fn a_link_on_the_way_to_an_item_fails_that_item_and_the_job_goes_on() {
    let root = results();
    let src = root.path().join("src");
    symlink(src.join("c"), src.join("link")).expect("link");
    symlink("b.txt", src.join("a/to-b")).expect("link");

    let (ended, paths) = packed(&root, &["link/f.txt", "a/to-b", "link/d/e.txt", "a/b.txt"]);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.skipped, 1, "a link as the item itself is skipped");
    let failed = ended.errors.iter().map(|error| error.path.as_str()).collect::<Vec<_>>();
    assert_eq!(failed, ["link/d/e.txt", "link/f.txt"]);
    assert_eq!(paths, ["a/b.txt"]);
}

#[test]
fn a_folder_on_the_way_swapped_for_a_link_while_the_items_are_found_fails_its_items() {
    let root = results();
    put(root.path(), "outside/d/e.txt", b"secret");
    let (folder, aside, outside) = (
        root.path().join("src/c"),
        root.path().join("src/c.aside"),
        root.path().join("outside"),
    );
    let hooks = Hooks {
        before_open: Some(Arc::new(move |now| {
            if now == "c" {
                fs::rename(&folder, &aside).expect("moved aside");
                symlink(&outside, &folder).expect("link");
            }
        })),
        ..Hooks::default()
    };

    let request = pack(
        root.path(),
        &["c/d/e.txt", "a/b.txt"],
        "out.zip",
        options(PackFormat::Zip, PackLevel::Normal),
    );
    let ended = Rig::with(hooks, |_, _| {}).run(request);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(
        ended.errors.iter().map(|error| error.path.as_str()).collect::<Vec<_>>(),
        ["c/d/e.txt"]
    );
    let seen = archived(&root, "out.zip", ArchiveFormat::Zip);
    assert_eq!(the_paths(&seen), ["a/b.txt"]);
}

#[test]
fn a_folder_on_the_way_swapped_for_a_link_after_the_items_were_planned_is_not_read_through() {
    let root = results();
    put(root.path(), "outside/b.txt", b"secret");
    let (folder, aside, outside) = (
        root.path().join("src/a"),
        root.path().join("src/a.aside"),
        root.path().join("outside"),
    );
    // The archive is being made: the items are planned and none of them was read yet.
    let hooks = Hooks {
        after_temp: Some(Arc::new(move |_| {
            fs::rename(&folder, &aside).expect("moved aside");
            symlink(&outside, &folder).expect("link");
        })),
        ..Hooks::default()
    };

    let request = pack(
        root.path(),
        &["a/b.txt", "z.txt"],
        "out.zip",
        options(PackFormat::Zip, PackLevel::Normal),
    );
    let ended = Rig::with(hooks, |_, _| {}).run(request);

    assert!(matches!(ended.outcome, Outcome::Failed { .. }), "{:?}", ended.outcome);
    let packed = fs::read(root.path().join("dst/out.zip")).ok();
    assert!(
        packed.is_none_or(|bytes| !bytes.windows(6).any(|window| window == b"secret")),
        "what the link leads to was read"
    );
    assert_eq!(
        fs::read_dir(root.path().join("dst")).expect("dir").count(),
        0,
        "no archive and no temporary file is left"
    );
}

#[test]
fn the_temporary_file_is_for_its_owner_alone_and_the_archive_gets_the_mode_of_a_new_file() {
    let root = scene();
    // What a plain new file gets here (the umask is the process's).
    let plain = root.path().join("dst/plain");
    fs::write(&plain, b"").expect("file");
    let normal = fs::metadata(&plain).expect("metadata").permissions().mode() & 0o777;
    fs::remove_file(&plain).expect("removed");
    let (hooks, seen) = temp_mode_hooks();

    let ended = Rig::with(hooks, |_, _| {}).run(zip(root.path()));

    assert_eq!(ended.outcome, Outcome::Done);
    let mode = seen.lock().expect("lock").expect("the hook ran");
    assert_eq!(mode & 0o077, 0, "{mode:o}");
    let made = fs::metadata(root.path().join("dst/out.zip")).expect("metadata");
    assert_eq!(made.permissions().mode() & 0o777, normal);
    // Nothing but the archive is left, the probe that read the mode of a new file included.
    assert_eq!(fs::read_dir(root.path().join("dst")).expect("dir").count(), 1);
}
