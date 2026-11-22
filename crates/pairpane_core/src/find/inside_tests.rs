//! Names inside archives.

use pretty_assertions::assert_eq;

use crate::archive::testing::{Member, SevenZ, gzip, project, tar_of, xz, zip_members, zip_of};
use crate::model::{EntryKind, FindCriteria};

use super::testing::{named, put, search, temp};

fn inside(names: &str) -> FindCriteria {
    FindCriteria {
        archives: true,
        ..named(names)
    }
}

#[test]
fn the_names_inside_zip_tar_and_7z_files_are_results_with_the_archive_and_the_entry() {
    let root = temp();
    put(root.path(), "pack.zip", &zip_of(&project()));
    put(root.path(), "pack.tar", &tar_of(&project()));
    put(root.path(), "sub/pack.7z", &SevenZ::default().of(&project()));
    put(root.path(), "pack.tar.xz", &xz(&tar_of(&project())));

    let found = search(root.path(), inside("util.rs"));

    assert_eq!(
        found.paths(),
        [
            "pack.tar>src/lib/util.rs",
            "pack.tar.xz>src/lib/util.rs",
            "pack.zip>src/lib/util.rs",
            "sub/pack.7z>src/lib/util.rs",
        ]
    );
    assert_eq!(found.done(), (4, 0, false, false));
}

#[test]
fn an_entry_says_what_it_is_and_its_size_and_date() {
    let root = temp();
    put(root.path(), "pack.zip", &zip_of(&project()));

    let hits = search(root.path(), inside("src;main.rs")).hits();

    assert_eq!(hits.len(), 2);
    assert_eq!(
        (hits[0].inner.as_deref(), hits[0].kind, hits[0].size),
        (Some("src"), EntryKind::Dir, 0)
    );
    assert_eq!(
        (
            hits[1].inner.as_deref(),
            hits[1].kind,
            hits[1].size,
            hits[1].path.as_str()
        ),
        (Some("src/main.rs"), EntryKind::File, 13, "pack.zip")
    );
    assert_eq!(hits[1].modified, Some(crate::archive::testing::MOMENT));
    assert!(hits.iter().all(|hit| !hit.link));
}

#[test]
fn without_the_option_archives_are_files_like_any_other() {
    let root = temp();
    put(root.path(), "pack.zip", &zip_of(&project()));

    let found = search(root.path(), named("*.rs;*.zip"));

    assert_eq!(found.paths(), ["pack.zip"]);
}

#[test]
fn the_archive_itself_is_a_result_when_its_name_fits() {
    let root = temp();
    put(root.path(), "pack.zip", &zip_of(&project()));

    assert_eq!(
        search(root.path(), inside("pack*;README")).paths(),
        ["pack.zip", "pack.zip>README"]
    );
}

#[test]
fn size_and_path_tests_apply_to_entries() {
    let root = temp();
    put(root.path(), "pack.zip", &zip_of(&project()));

    let mut criteria = inside("*");
    criteria.min_size = Some(14);
    assert_eq!(
        search(root.path(), criteria).paths(),
        ["pack.zip", "pack.zip>src/lib/util.rs"]
    );

    let mut criteria = inside("pack.zip/src/*.rs");
    criteria.name_path = true;
    assert_eq!(
        search(root.path(), criteria).paths(),
        ["pack.zip>src/lib/util.rs", "pack.zip>src/main.rs"]
    );
}

#[test]
fn hidden_entries_are_skipped_with_the_option() {
    let root = temp();
    put(
        root.path(),
        "pack.zip",
        &zip_of(&[(".git/config", b"x"), ("a.txt", b"x")]),
    );

    let mut criteria = inside("*");
    assert_eq!(
        search(root.path(), criteria.clone()).paths(),
        ["pack.zip", "pack.zip>.git/config", "pack.zip>a.txt"]
    );

    criteria.skip_hidden = true;
    assert_eq!(search(root.path(), criteria).paths(), ["pack.zip", "pack.zip>a.txt"]);
}

#[test]
fn rar_and_gzip_files_are_not_looked_into_and_a_broken_archive_is_counted() {
    let root = temp();
    put(root.path(), "a.rar", b"Rar!\x1a\x07\x00 not read");
    put(root.path(), "b.gz", &gzip(b"inside"));
    put(root.path(), "broken.zip", b"PK\x03\x04 this is not a zip");
    put(root.path(), "plain.txt", b"x");

    let found = search(root.path(), inside("*.txt;inside*;*.rar;*.gz"));

    assert_eq!(found.paths(), ["a.rar", "b.gz", "plain.txt"]);
    assert_eq!(found.done().1, 1);
}

#[test]
fn an_entry_with_an_unsafe_name_is_counted_and_never_a_result() {
    let root = temp();
    let members = [Member::stored("../escape.txt", b"x"), Member::stored("ok.txt", b"x")];
    put(root.path(), "evil.zip", &zip_members(&members));

    let found = search(root.path(), inside("*.txt"));

    assert_eq!(found.paths(), ["evil.zip>ok.txt"]);
    assert_eq!(found.done().1, 1);
}

#[cfg(unix)]
#[test]
fn a_link_to_an_archive_is_not_opened() {
    let root = temp();
    let outside = temp();
    put(outside.path(), "pack.zip", &zip_of(&project()));
    std::os::unix::fs::symlink(outside.path().join("pack.zip"), root.path().join("link.zip")).expect("link");

    let found = search(root.path(), inside("*.rs"));

    assert!(found.paths().is_empty());
}
