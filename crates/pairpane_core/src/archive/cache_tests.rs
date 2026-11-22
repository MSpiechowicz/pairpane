//! The cache of indexes: the least recently used one goes, and a changed file is a different key.

use std::fs;
use std::sync::Arc;

use crate::model::ArchiveFormat;

use super::cache::{CACHE_SIZE, Fingerprint, IndexCache};
use super::index::{ArchiveIndex, Builder};

fn index() -> Arc<ArchiveIndex> {
    Arc::new(Builder::new(ArchiveFormat::Tar).finish(false))
}

fn fingerprint(folder: &std::path::Path, name: &str) -> Fingerprint {
    let path = folder.join(name);

    Fingerprint::of(&path, &fs::metadata(&path).expect("metadata"))
}

fn files(names: &[&str]) -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("a folder");

    for name in names {
        fs::write(folder.path().join(name), name.as_bytes()).expect("write");
    }

    folder
}

#[test]
fn an_index_is_found_by_its_fingerprint() {
    let folder = files(&["a.tar"]);
    let cache = IndexCache::new();
    let stored = index();

    assert!(cache.get(&fingerprint(folder.path(), "a.tar")).is_none());
    cache.put(fingerprint(folder.path(), "a.tar"), Arc::clone(&stored));

    assert!(Arc::ptr_eq(
        &cache.get(&fingerprint(folder.path(), "a.tar")).expect("kept"),
        &stored
    ));
}

#[test]
fn the_least_recently_used_index_goes_first() {
    let names = ["a", "b", "c", "d", "e"];
    let folder = files(&names);
    let cache = IndexCache::new();

    for name in &names[..CACHE_SIZE] {
        cache.put(fingerprint(folder.path(), name), index());
    }
    // Using `a` makes `b` the oldest.
    assert!(cache.get(&fingerprint(folder.path(), "a")).is_some());
    cache.put(fingerprint(folder.path(), "e"), index());

    assert!(cache.get(&fingerprint(folder.path(), "b")).is_none());
    for name in ["a", "c", "d", "e"] {
        assert!(cache.get(&fingerprint(folder.path(), name)).is_some(), "{name}");
    }
}

#[test]
fn a_file_that_changed_is_not_served_from_the_cache() {
    let folder = files(&["a.tar"]);
    let cache = IndexCache::new();
    cache.put(fingerprint(folder.path(), "a.tar"), index());

    fs::write(folder.path().join("a.tar"), b"a longer content than before").expect("write");

    assert!(cache.get(&fingerprint(folder.path(), "a.tar")).is_none());

    // Keeping the new state drops the old one of the same file.
    cache.put(fingerprint(folder.path(), "a.tar"), index());
    assert!(cache.get(&fingerprint(folder.path(), "a.tar")).is_some());
}

#[test]
fn forgetting_an_archive_drops_its_index() {
    let folder = files(&["a.tar", "b.tar"]);
    let cache = IndexCache::new();
    cache.put(fingerprint(folder.path(), "a.tar"), index());
    cache.put(fingerprint(folder.path(), "b.tar"), index());

    cache.forget(&folder.path().join("a.tar"));

    assert!(cache.get(&fingerprint(folder.path(), "a.tar")).is_none());
    assert!(cache.get(&fingerprint(folder.path(), "b.tar")).is_some());
}
