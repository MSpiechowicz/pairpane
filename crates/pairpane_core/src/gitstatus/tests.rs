//! What a status says about real repositories (made with gix in a temporary folder), the caps, and the errors.

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use gix::index::entry::Stage;

use super::fixture::{Repo, Staged, pairs};
use super::*;
use crate::model::GitMark::{Conflicted, Ignored, Modified, Staged as Added, Untracked};

fn read(folder: &Path) -> GitStatus {
    status(folder, Limits::default(), &AtomicBool::new(false)).expect("a status")
}

fn marks_of(folder: &Path) -> Vec<(String, crate::model::GitMark)> {
    read(folder)
        .marks
        .into_iter()
        .map(|entry| (entry.name, entry.mark))
        .collect()
}

fn owned(list: &[(&str, crate::model::GitMark)]) -> Vec<(String, crate::model::GitMark)> {
    list.iter().map(|(name, mark)| ((*name).to_owned(), *mark)).collect()
}

/// A repository with one commit whose files are on disk and in the index, unchanged.
pub(super) fn clean(files: &[(&str, &[u8])]) -> Repo {
    let repo = Repo::new();
    repo.commit(files);

    repo
}

#[test]
fn every_mark_of_a_file_is_told_and_a_clean_file_has_none() {
    let repo = clean(&[
        (".gitignore", b"*.log\n"),
        ("a.txt", b"a"),
        ("b.txt", b"b"),
        ("both.txt", b"head"),
        ("c.txt", b"base"),
        ("same.txt", b"same"),
    ]);
    // The index: b and both staged with new content, a new file staged, c in conflict.
    repo.write_index(&[
        Staged::file(".gitignore", b"*.log\n"),
        Staged::file("a.txt", b"a"),
        Staged::file("b.txt", b"b staged"),
        Staged::file("both.txt", b"both staged"),
        Staged::conflict("c.txt", b"base", Stage::Base),
        Staged::conflict("c.txt", b"ours", Stage::Ours),
        Staged::conflict("c.txt", b"theirs", Stage::Theirs),
        Staged::file("new.txt", b"new"),
        Staged::file("same.txt", b"same"),
    ]);
    repo.put("a.txt", b"a, changed on disk");
    repo.put("b.txt", b"b staged");
    repo.put("both.txt", b"both changed on disk again");
    repo.put("c.txt", b"ours");
    repo.put("new.txt", b"new");
    repo.put("u.txt", b"untracked");
    repo.put("skip.log", b"ignored");

    let answer = read(repo.root());

    assert!(answer.repo);
    assert_eq!(
        pairs(&answer.marks),
        vec![
            ("a.txt", Modified),
            ("b.txt", Added),
            // Staged and modified is modified.
            ("both.txt", Modified),
            ("c.txt", Conflicted),
            ("new.txt", Added),
            ("skip.log", Ignored),
            ("u.txt", Untracked),
        ]
    );
}

#[test]
fn names_that_are_not_ascii_are_told_as_the_folder_lists_them() {
    let repo = clean(&[("caf\u{e9}.txt", b"c"), ("\u{65e5}\u{672c}/x.txt", b"x")]);
    repo.put("caf\u{e9}.txt", b"c, changed");
    repo.put("\u{fc}ber.txt", b"u");

    assert_eq!(
        marks_of(repo.root()),
        owned(&[("caf\u{e9}.txt", Modified), ("\u{fc}ber.txt", Untracked)])
    );
}

#[test]
fn a_file_removed_from_the_work_tree_is_modified_and_one_the_index_dropped_is_staged() {
    let repo = clean(&[("gone.txt", b"g"), ("kept.txt", b"k"), ("dropped.txt", b"d")]);
    repo.write_index(&[Staged::file("gone.txt", b"g"), Staged::file("kept.txt", b"k")]);
    repo.remove("gone.txt");

    assert_eq!(
        marks_of(repo.root()),
        owned(&[("dropped.txt", Added), ("gone.txt", Modified)])
    );
}

#[test]
fn a_repository_with_no_commit_has_every_indexed_file_staged() {
    let repo = Repo::new();
    repo.write_index(&[Staged::file("a.txt", b"a")]);
    repo.put("a.txt", b"a");
    repo.put("b.txt", b"b");

    assert_eq!(marks_of(repo.root()), owned(&[("a.txt", Added), ("b.txt", Untracked)]));
}

#[test]
fn a_folder_is_modified_when_anything_below_it_changed_and_ignored_when_it_is_ignored() {
    let repo = clean(&[
        (".gitignore", b"cache/\n*.tmp\n"),
        ("edited/deep/x.txt", b"x"),
        ("staged/y.txt", b"y"),
        ("clean/z.txt", b"z"),
        ("only-ignored-inside/keep.txt", b"k"),
        ("conflict/c.txt", b"c"),
    ]);
    repo.write_index(&[
        Staged::conflict("conflict/c.txt", b"c", Stage::Base),
        Staged::conflict("conflict/c.txt", b"mine", Stage::Ours),
        Staged::file(".gitignore", b"cache/\n*.tmp\n"),
        Staged::file("clean/z.txt", b"z"),
        Staged::file("edited/deep/x.txt", b"x"),
        Staged::file("only-ignored-inside/keep.txt", b"k"),
        Staged::file("staged/y.txt", b"y two"),
    ]);
    repo.put("edited/deep/x.txt", b"x, edited");
    repo.put("staged/y.txt", b"y two");
    repo.put("fresh/new.txt", b"untracked inside a folder of its own");
    repo.put("cache/data.bin", b"ignored folder");
    repo.put("only-ignored-inside/scratch.tmp", b"ignored file in a tracked folder");
    repo.put("conflict/c.txt", b"mine");

    assert_eq!(
        marks_of(repo.root()),
        owned(&[
            ("cache", Ignored),
            // Conflicted files make their folder modified, like any other change.
            ("conflict", Modified),
            ("edited", Modified),
            ("fresh", Modified),
            ("staged", Modified),
        ])
    );
}

#[test]
fn only_the_subtree_of_the_folder_is_looked_at_and_the_marks_are_its_children() {
    let repo = clean(&[
        ("top.txt", b"t"),
        ("sub/in.txt", b"i"),
        ("sub/deeper/in2.txt", b"i2"),
        ("other/o.txt", b"o"),
    ]);
    repo.put("top.txt", b"t changed");
    repo.put("other/o.txt", b"o changed");
    repo.put("other/untracked.txt", b"u");
    repo.put("sub/in.txt", b"i changed");
    repo.put("sub/deeper/new.txt", b"n");
    repo.put("sub/added.txt", b"a");

    assert_eq!(
        marks_of(&repo.path("sub")),
        owned(&[("added.txt", Untracked), ("deeper", Modified), ("in.txt", Modified)])
    );
    assert_eq!(marks_of(&repo.path("sub/deeper")), owned(&[("new.txt", Untracked)]));
    // The root sees its own children, with the folders below them as folders.
    assert_eq!(
        marks_of(repo.root()),
        owned(&[("other", Modified), ("sub", Modified), ("top.txt", Modified)])
    );
}

#[test]
fn a_folder_that_is_untracked_or_ignored_as_a_whole_marks_each_of_its_children() {
    let repo = clean(&[(".gitignore", b"build/\n"), ("keep.txt", b"k")]);
    repo.put("fresh/one.txt", b"1");
    repo.put("fresh/two/three.txt", b"3");
    repo.put("build/out.bin", b"o");
    repo.put("build/sub/more.bin", b"m");

    assert_eq!(
        marks_of(&repo.path("fresh")),
        owned(&[("one.txt", Untracked), ("two", Modified)])
    );
    assert_eq!(
        marks_of(&repo.path("build")),
        owned(&[("out.bin", Ignored), ("sub", Ignored)])
    );
    assert_eq!(marks_of(&repo.path("build/sub")), owned(&[("more.bin", Ignored)]));
}

#[test]
fn a_folder_name_with_pattern_characters_is_taken_literally() {
    let repo = clean(
        &[("a[1]/x.txt", b"x"), ("a1/y.txt", b"y"), ("*/z.txt", b"z")].map(|(path, content)| (path, content as &[u8])),
    );
    repo.put("a[1]/x.txt", b"changed");
    repo.put("a1/new.txt", b"untracked in the other folder");

    assert_eq!(marks_of(&repo.path("a[1]")), owned(&[("x.txt", Modified)]));
    assert_eq!(marks_of(&repo.path("*")), vec![]);
}

#[test]
fn a_folder_outside_any_repository_is_not_a_repo() {
    let folder = tempfile::tempdir().expect("a folder");
    fs::write(folder.path().join("a.txt"), b"a").expect("a file");

    let answer = read(folder.path());

    assert_eq!(
        answer,
        GitStatus {
            repo: false,
            marks: vec![]
        }
    );
}

/// The marks of `folder` when the search for a repository must stop at `home`.
fn marks_with_home(folder: &Path, home: Option<&Path>) -> Option<Vec<(String, crate::model::GitMark)>> {
    let answer = scan::status_below(
        folder,
        home.map(Path::to_path_buf),
        Limits::default(),
        &AtomicBool::new(false),
    )
    .expect("a status");

    answer
        .repo
        .then(|| answer.marks.into_iter().map(|entry| (entry.name, entry.mark)).collect())
}

#[test]
fn the_search_for_a_repository_stops_at_the_home_folder_and_includes_it() {
    // A repository above the home folder is not the home folder's.
    let outer = clean(&[("top.txt", b"t")]);
    let home = outer.path("home");
    fs::create_dir_all(home.join("work")).expect("folders");
    fs::write(home.join("work/file.txt"), b"f").expect("a file");

    assert_eq!(marks_with_home(&home.join("work"), Some(&home)), None);
    assert!(
        marks_with_home(&home.join("work"), None).is_some(),
        "with no home the search goes on up"
    );

    // A repository inside the home folder, or at it, is found.
    gix::init(home.join("work")).expect("a repository in the home folder");
    assert_eq!(
        marks_with_home(&home.join("work"), Some(&home)),
        Some(owned(&[("file.txt", Untracked)]))
    );
    let dotfiles = clean(&[("rc", b"r")]);
    fs::create_dir_all(dotfiles.path("config")).expect("a folder");
    assert!(marks_with_home(&dotfiles.path("config"), Some(dotfiles.root())).is_some());
}

#[cfg(unix)]
#[test]
fn a_link_to_a_folder_of_a_repository_is_read_as_the_folder_it_leads_to() {
    let repo = clean(&[("sub/in.txt", b"i")]);
    repo.put("sub/in.txt", b"changed");
    let elsewhere = tempfile::tempdir().expect("a folder");
    let link = elsewhere.path().join("link");
    std::os::unix::fs::symlink(repo.path("sub"), &link).expect("a link");

    assert_eq!(marks_of(&link), owned(&[("in.txt", Modified)]));
}

#[test]
fn the_git_folder_itself_is_not_a_place_with_marks_and_is_not_marked() {
    let repo = clean(&[("a.txt", b"a")]);
    repo.put("b.txt", b"b");

    assert!(!read(repo.git_dir()).repo);
    assert!(!read(&repo.git_dir().join("refs")).repo);
    assert!(marks_of(repo.root()).iter().all(|(name, _)| name != ".git"));
}

#[test]
fn a_bare_repository_has_no_work_tree() {
    let folder = tempfile::tempdir().expect("a folder");
    let bare = folder.path().join("project.git");
    gix::init_bare(&bare).expect("a bare repository");

    assert!(!read(&bare).repo);
    assert!(!read(&bare.join("refs")).repo);
}

#[test]
fn a_git_file_that_points_to_a_worktree_of_another_repository_works() {
    let main = clean(&[("a.txt", b"a"), ("b.txt", b"b")]);
    let linked = main.dir.path().join("linked");
    let admin = main.git_dir().join("worktrees").join("linked");
    fs::create_dir_all(&linked).expect("the worktree folder");
    fs::create_dir_all(&admin).expect("the admin folder");
    fs::write(linked.join(".git"), format!("gitdir: {}\n", admin.display())).expect("the git file");
    fs::write(admin.join("gitdir"), format!("{}\n", linked.join(".git").display())).expect("gitdir");
    fs::write(admin.join("commondir"), "../..\n").expect("commondir");
    fs::copy(main.git_dir().join("HEAD"), admin.join("HEAD")).expect("HEAD");
    fs::copy(main.git_dir().join("index"), admin.join("index")).expect("the index");
    // The branch `HEAD` names is shared, in the common directory.
    fs::write(linked.join("a.txt"), b"a").expect("a");
    fs::write(linked.join("b.txt"), b"b, changed").expect("b");
    fs::write(linked.join("c.txt"), b"c").expect("c");

    assert_eq!(marks_of(&linked), owned(&[("b.txt", Modified), ("c.txt", Untracked)]));
}

#[test]
fn a_git_file_that_points_nowhere_is_no_repository() {
    let folder = tempfile::tempdir().expect("a folder");
    fs::write(folder.path().join(".git"), "gitdir: ../does-not-exist\n").expect("the git file");
    fs::write(folder.path().join("a.txt"), b"a").expect("a file");

    assert!(!read(folder.path()).repo);
}

#[test]
fn a_submodule_is_never_opened_and_has_no_mark_of_its_own() {
    let repo = Repo::new();
    repo.commit_with_submodules(&[("a.txt", b"a")], &["vendor/lib"]);
    // The submodule's own repository has an untracked file, and a modified one.
    let inner = repo.path("vendor/lib");
    fs::create_dir_all(&inner).expect("the submodule folder");
    gix::init(&inner).expect("the submodule's repository");
    fs::write(inner.join("loose.txt"), b"untracked in the submodule").expect("a file");

    assert_eq!(marks_of(repo.root()), vec![]);
    assert_eq!(marks_of(&repo.path("vendor")), vec![]);
    // Browsing into the submodule is browsing its repository, under the same restrictions.
    assert_eq!(marks_of(&inner), owned(&[("loose.txt", Untracked)]));
}

#[test]
fn an_untracked_nested_repository_is_an_untracked_folder_and_is_not_entered() {
    let repo = clean(&[("a.txt", b"a")]);
    let nested = repo.path("nested");
    fs::create_dir_all(&nested).expect("a folder");
    gix::init(&nested).expect("a nested repository");
    fs::write(nested.join("inside.txt"), b"i").expect("a file");

    assert_eq!(marks_of(repo.root()), owned(&[("nested", Modified)]));
}

#[test]
fn reading_a_status_changes_nothing_in_the_repository() {
    let repo = clean(&[("a.txt", b"a"), ("b.txt", b"b")]);
    repo.put("a.txt", b"a changed");
    repo.put("c.txt", b"c");
    let before = snapshot(repo.git_dir());

    let _ = read(repo.root());

    assert_eq!(snapshot(repo.git_dir()), before);
}

/// Every file below `folder` with its bytes, in path order.
fn snapshot(folder: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next).expect("a folder") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push((path.clone(), fs::read(&path).expect("bytes")));
            }
        }
    }
    found.sort();

    found
}
