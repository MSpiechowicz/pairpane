//! The repository that is looked at is the repository gix opens: what a `.git` file, a `commondir`, a `gitdir` and a list of
//! alternates say is read as gix reads it, a link is followed hop by hop, and what gix opened is checked against what was
//! looked at.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::fixture::{Repo, pairs, within};
use super::locate::{Layout, is_head};
use super::tests::clean;
use super::*;
use crate::model::GitMark::Untracked;

/// The status of `folder` on a thread of its own, which has to come in time (a read of a named pipe waits for ever).
fn in_time(folder: &Path) -> GitStatus {
    let folder = folder.to_path_buf();

    within(10, move || status(&folder, Limits::default(), &AtomicBool::new(false))).expect("a status")
}

/// A git folder that is all gix asks for: a `HEAD`, `objects` and `refs`.
fn git_folder(dir: &Path) {
    fs::create_dir_all(dir.join("objects")).expect("objects");
    fs::create_dir_all(dir.join("refs/heads")).expect("refs");
    fs::write(dir.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD");
}

#[test]
fn the_content_of_a_head_is_a_symbolic_ref_below_refs_or_an_object_id() {
    let id40 = "0123456789abcdef0123456789abcdef01234567";
    let id64 = id40.repeat(2)[..64].to_owned();

    for good in [
        "ref: refs/heads/main\n",
        "ref:refs/heads/x",
        "ref:   refs/tags/v1\r\n",
        id40,
        &id64,
    ] {
        assert!(is_head(good.as_bytes()), "{good:?}");
    }
    for bad in [
        "",
        "garbage\n",
        "ref: HEAD",
        "ref: heads/main",
        "ref: ",
        "0123456789abcdef",
        "0123456789abcdef0123456789abcdef0123456g",
    ] {
        assert!(!is_head(bad.as_bytes()), "{bad:?}");
    }
}

#[test]
fn a_folder_whose_head_is_not_a_ref_or_an_id_is_no_git_folder_and_the_search_goes_on_to_its_parent() {
    let outer = clean(&[("sub/file.txt", b"outer")]);
    outer.put("sub/file.txt", b"changed");
    let inner = outer.path("sub/.git");
    git_folder(&inner);

    // As it is the folder is a repository of its own, and its file is untracked there.
    assert_eq!(pairs(&in_time(&outer.path("sub")).marks), vec![("file.txt", Untracked)]);

    // With a `HEAD` that is neither, git goes on to the parent, where the file is tracked, and changed.
    fs::write(inner.join("HEAD"), "garbage\n").expect("HEAD");
    assert_eq!(
        pairs(&in_time(&outer.path("sub")).marks),
        vec![("file.txt", crate::model::GitMark::Modified)]
    );
}

#[test]
fn what_gix_opened_must_be_what_was_looked_at() {
    let repo = Repo::new();
    let opened = super::open::open(&repo.path(".git"))
        .expect("opened")
        .expect("a repository");
    let layout = |dot_git: &str, git_dir: &str, common_dir: &str, workdir: &Path| Layout {
        dot_git: repo.path(dot_git),
        git_dir: repo.path(git_dir),
        common_dir: repo.path(common_dir),
        workdir: workdir.to_path_buf(),
    };

    assert!(layout(".git", ".git", ".git", repo.root()).is_opened(&opened));
    // Dots are read away.
    assert!(layout(".git", "x/../.git", ".git/.", &repo.path("x/..")).is_opened(&opened));

    assert!(!layout(".git", "other.git", ".git", repo.root()).is_opened(&opened));
    assert!(!layout(".git", ".git", "other.git", repo.root()).is_opened(&opened));
    assert!(!layout(".git", ".git", ".git", &repo.path("evil")).is_opened(&opened));
    assert!(!layout(".git", ".git", ".git", repo.path("..").as_path()).is_opened(&opened));
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::view::testing::fifo;

    fn is_repo(folder: &Path) -> bool {
        in_time(folder).repo
    }

    /// `main` has a linked work tree in a folder of its own, whose admin folder is returned with the folder.
    fn linked(main: &Repo, outside: &Path) -> (PathBuf, PathBuf) {
        let (linked, admin) = main.linked_worktree_at(outside.join("linked"), "linked");
        fs::write(linked.join("a.txt"), b"a").expect("a file");

        (linked, admin)
    }

    #[test]
    fn a_git_folder_that_names_another_work_tree_with_commondir_and_gitdir_is_no_repository() {
        let outer = clean(&[("a.txt", b"a")]);
        let evil = outer.path("evil");
        git_folder(&evil.join(".git"));
        fs::write(outer.path(".gitignore"), "evil/secret*\n").expect("rules");
        fs::write(evil.join("secret.txt"), b"s").expect("a file");

        // Without the two files it is a repository of its own.
        let answer = in_time(&evil);
        assert!(answer.repo);
        assert_eq!(pairs(&answer.marks), vec![("secret.txt", Untracked)]);

        // With them gix takes the work tree from the content of `gitdir`: the outer one.
        fs::write(evil.join(".git/commondir"), ".\n").expect("commondir");
        fs::write(evil.join(".git/gitdir"), format!("{}\n", outer.path(".git").display())).expect("gitdir");
        assert!(!is_repo(&evil));

        // A `gitdir` that names this very folder, spelled in any way, is the work tree that holds it.
        for text in [
            format!("{}\n", evil.join(".git").display()),
            format!("{}/./../evil/.git\n", evil.display()),
            "../.git\n".to_owned(),
        ] {
            fs::write(evil.join(".git/gitdir"), &text).expect("gitdir");
            let answer = in_time(&evil);

            assert!(answer.repo, "{text:?}");
            assert_eq!(pairs(&answer.marks), vec![("secret.txt", Untracked)], "{text:?}");
        }
    }

    #[test]
    fn a_gitdir_pointer_is_the_text_gix_reads_with_its_leading_spaces_and_its_lines() {
        let folder = tempfile::tempdir().expect("a folder");
        let work = folder.path().join("w");
        git_folder(&work.join("evil"));
        let hostile = work.join(" evil");
        git_folder(&hostile);
        fs::remove_file(hostile.join("HEAD")).expect("removed");
        fifo(&hostile, "HEAD");

        // Two spaces: gix opens ` evil`, whose `HEAD` is a pipe, and not `evil`.
        fs::write(work.join(".git"), "gitdir:  evil\n").expect("a git file");
        assert!(!is_repo(&work));

        // The folder is fine, and then the pointer is followed to it.
        fs::remove_file(hostile.join("HEAD")).expect("removed");
        fs::write(hostile.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD");
        assert!(is_repo(&work));

        // The whole text is the path, up to the end: a second line is not cut off.
        fs::write(work.join(".git"), "gitdir: evil\nmore\n").expect("a git file");
        assert!(!is_repo(&work));
        fs::write(work.join(".git"), "gitdir: evil   \n\n").expect("a git file");
        assert!(is_repo(&work));
    }

    #[test]
    fn a_commondir_is_read_with_the_white_space_at_its_end_cut_off_and_nothing_else() {
        let main = clean(&[("a.txt", b"a")]);
        let outside = tempfile::tempdir().expect("a folder");
        let (linked, admin) = linked(&main, outside.path());

        for (text, repo) in [
            ("../..\n", true),
            ("../.. \r\n\n", true),
            (" ../..\n", false),
            ("\n", false),
        ] {
            fs::write(admin.join("commondir"), text).expect("commondir");

            assert_eq!(is_repo(&linked), repo, "{text:?}");
        }
    }

    #[test]
    fn the_alternates_of_every_depth_are_joined_to_the_objects_of_the_repository_as_gix_joins_them() {
        // The second list says `b`, and gix reads it from the objects folder of the repository, whatever list it is in; the
        // list of `c` has a `\r`, which is a part of the name.
        for (first, second, hostile) in [
            ("a\n", "b\n", "objects/b/info/alternates"),
            ("a\n", "../c\n", "c/info/alternates"),
            ("a\r\n", "x\n", "objects/a\r/info/alternates"),
        ] {
            let repo = clean(&[("a.txt", b"a")]);
            let objects = repo.git_dir().join("objects");
            fs::create_dir_all(objects.join("info")).expect("a folder");
            fs::write(objects.join("info/alternates"), first).expect("a list");
            for name in ["a", "a\r"] {
                fs::create_dir_all(objects.join(name).join("info")).expect("a folder");
            }
            fs::write(objects.join("a/info/alternates"), second).expect("a list");
            let pipe = repo.git_dir().join(hostile);
            fs::create_dir_all(pipe.parent().expect("a parent")).expect("folders");
            fifo(
                pipe.parent().expect("a parent"),
                pipe.file_name().expect("a name").to_str().expect("text"),
            );

            assert!(!is_repo(repo.root()), "{hostile:?}");
        }
    }

    #[test]
    fn a_dot_git_that_is_a_link_to_an_automounted_network_folder_ends_the_search() {
        let outer = clean(&[("sub/file.txt", b"f")]);
        for (target, repo) in [
            ("/net/host/x/.git", false),
            ("/System/Volumes/Data/Network/x", false),
            ("/srv-that-is-not-there/.git", true),
        ] {
            let _ = fs::remove_file(outer.path("sub/.git"));
            symlink(target, outer.path("sub/.git")).expect("a link");

            // What is not a network place leads nowhere and the search goes on to the parent, which is a repository.
            assert_eq!(is_repo(&outer.path("sub")), repo, "{target}");
        }
    }

    #[test]
    fn a_gitdir_pointer_through_a_link_dots_or_a_network_place_is_refused_and_ends_the_search() {
        let outer = clean(&[("sub/file.txt", b"f")]);
        let sub = outer.path("sub");
        symlink("/net/host", sub.join("hop")).expect("a link");
        symlink("hop", sub.join("second")).expect("a link");
        symlink("/srv-that-is-not-there", sub.join("away")).expect("a link");

        for (pointer, repo) in [
            ("hop/repo.git", false),
            ("second/repo.git", false),
            ("/srv/../net/host/repo.git", false),
            ("/net/host/repo.git", false),
            ("away/repo.git", true),
            ("../does-not-exist", true),
        ] {
            fs::write(sub.join(".git"), format!("gitdir: {pointer}\n")).expect("a git file");

            assert_eq!(is_repo(&sub), repo, "{pointer}");
        }
    }

    #[test]
    fn a_link_for_info_exclude_or_attributes_is_followed_to_a_regular_local_file_and_otherwise_refused() {
        let elsewhere = tempfile::tempdir().expect("a folder");
        fs::write(elsewhere.path().join("rules"), "*.tmp\n").expect("rules");
        fifo(elsewhere.path(), "pipe");
        fs::write(elsewhere.path().join("big"), vec![b'#'; 2 * 1024 * 1024]).expect("a big file");

        for name in ["exclude", "attributes"] {
            for (target, repo) in [
                (elsewhere.path().join("rules"), true),
                (elsewhere.path().join("missing"), true),
                (elsewhere.path().join("pipe"), false),
                (elsewhere.path().join("big"), false),
                (PathBuf::from("/dev/zero"), false),
                (PathBuf::from("/net/host/exclude"), false),
            ] {
                let repo_under_test = clean(&[("a.txt", b"a")]);
                repo_under_test.put("x.tmp", b"x");
                let info = repo_under_test.git_dir().join("info");
                fs::create_dir_all(&info).expect("a folder");
                let _ = fs::remove_file(info.join(name));
                symlink(&target, info.join(name)).expect("a link");

                let answer = in_time(repo_under_test.root());

                assert_eq!(answer.repo, repo, "{name} -> {}", target.display());
            }
        }

        // What the link leads to is read as the rules.
        let repo = clean(&[("a.txt", b"a")]);
        repo.put("x.tmp", b"x");
        fs::create_dir_all(repo.git_dir().join("info")).expect("a folder");
        let _ = fs::remove_file(repo.git_dir().join("info/exclude"));
        symlink(elsewhere.path().join("rules"), repo.git_dir().join("info/exclude")).expect("a link");
        assert_eq!(
            pairs(&in_time(repo.root()).marks),
            vec![("x.tmp", crate::model::GitMark::Ignored)]
        );
    }

    #[test]
    fn a_head_that_is_a_link_to_a_local_file_is_followed_as_gix_follows_it() {
        let elsewhere = tempfile::tempdir().expect("a folder");
        fs::write(elsewhere.path().join("head"), "ref: refs/heads/main\n").expect("HEAD");
        let repo = clean(&[("a.txt", b"a")]);
        fs::remove_file(repo.git_dir().join("HEAD")).expect("removed");
        symlink(elsewhere.path().join("head"), repo.git_dir().join("HEAD")).expect("a link");

        assert!(is_repo(repo.root()));

        fs::remove_file(repo.git_dir().join("HEAD")).expect("removed");
        fifo(elsewhere.path(), "pipe");
        symlink(elsewhere.path().join("pipe"), repo.git_dir().join("HEAD")).expect("a link");
        assert!(!is_repo(repo.root()));
    }

    #[test]
    fn a_link_for_a_structural_folder_of_the_repository_or_of_its_objects_is_no_repository() {
        let fan_out = |repo: &Repo| -> String {
            fs::read_dir(repo.git_dir().join("objects"))
                .expect("objects")
                .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                .find(|name| name.len() == 2 && name != "pack" && name != "info")
                .expect("a folder of loose objects")
        };

        for name in ["refs", "objects", "objects/pack", "objects/info", "FAN-OUT"] {
            let repo = clean(&[("a.txt", b"a")]);
            assert!(is_repo(repo.root()), "{name}: without the link");
            let name = if name == "FAN-OUT" {
                format!("objects/{}", fan_out(&repo))
            } else {
                name.to_owned()
            };
            let elsewhere = tempfile::tempdir().expect("a folder");
            fs::create_dir_all(repo.git_dir().join(&name)).expect("a folder");
            fs::rename(repo.git_dir().join(&name), elsewhere.path().join("moved")).expect("moved");
            symlink(elsewhere.path().join("moved"), repo.git_dir().join(&name)).expect("a link");

            assert!(!is_repo(repo.root()), "{name}");
        }
    }
}
