//! A sparse index (`git sparse-checkout set --cone --sparse-index`): a folder that is out of the work tree is one entry of the
//! index with the mode of a folder and the id of its tree, and the folder is as `HEAD` has it.

use std::path::Path;

use super::fixture::{Repo, Staged};
use super::tests::{clean, owned};
use super::*;
use crate::model::GitMark::{Modified, Staged as Added, Untracked};

fn marks(folder: &Path) -> Vec<(String, crate::model::GitMark)> {
    super::tests::marks_with(folder, Limits::default())
}

/// A repository whose `HEAD` has `a.txt`, `b/x.txt`, `b/y/z.txt` and `c/d.txt`, whose index is sparse: `b/` is one entry of
/// the folder, and `b` is not in the work tree.
fn sparse() -> Repo {
    let repo = clean(&[
        ("a.txt", b"a"),
        ("b/x.txt", b"x"),
        ("b/y/z.txt", b"z"),
        ("c/d.txt", b"d"),
    ]);
    let tree = repo.tree_of("b");
    repo.write_index(&[
        Staged::file("a.txt", b"a"),
        Staged::sparse("b/", tree),
        Staged::file("c/d.txt", b"d"),
    ]);
    std::fs::remove_dir_all(repo.path("b")).expect("b is out of the work tree");

    repo
}

#[test]
fn a_clean_repository_with_a_sparse_index_has_no_mark_for_the_folders_that_are_out_of_the_work_tree() {
    let repo = sparse();

    assert_eq!(marks(repo.root()), vec![]);
    assert_eq!(marks(&repo.path("c")), vec![]);
}

#[test]
fn a_folder_inside_a_sparse_directory_is_not_staged_as_deleted() {
    let repo = sparse();
    // A folder that was left on disk (or made again) where the index has the sparse entry.
    repo.put("b/y/leftover.txt", b"l");

    // Whatever the walk says of what is left, `HEAD`'s files below the sparse entry are not "deleted" (staged).
    for folder in ["b", "b/y"] {
        let answer = marks(&repo.path(folder));

        assert!(answer.iter().all(|(_, mark)| *mark != Added), "{folder}: {answer:?}");
    }
}

#[test]
fn a_change_next_to_a_sparse_directory_is_found_and_a_sparse_directory_that_is_not_the_one_of_head_is_staged() {
    let repo = sparse();
    repo.put("a.txt", b"changed");
    repo.put("new.txt", b"n");

    assert_eq!(
        marks(repo.root()),
        owned(&[("a.txt", Modified), ("new.txt", Untracked)])
    );

    // The index holds another tree for `b` than `HEAD` does: something below it is staged.
    let other = repo.tree_of("c");
    repo.write_index(&[
        Staged::file("a.txt", b"changed"),
        Staged::sparse("b/", other),
        Staged::file("c/d.txt", b"d"),
    ]);

    assert_eq!(
        marks(repo.root()),
        owned(&[("a.txt", Added), ("b", Added), ("new.txt", Untracked)])
    );
}

#[test]
fn the_sparse_directories_of_a_deep_folder_are_found_below_its_own_folders() {
    let repo = clean(&[("top/a.txt", b"a"), ("top/b/x.txt", b"x"), ("top/b/y/z.txt", b"z")]);
    let tree = repo.tree_of("top/b");
    repo.write_index(&[Staged::file("top/a.txt", b"a"), Staged::sparse("top/b/", tree)]);
    std::fs::remove_dir_all(repo.path("top/b")).expect("out of the work tree");

    assert_eq!(marks(repo.root()), vec![]);
    assert_eq!(marks(&repo.path("top")), vec![]);
}
