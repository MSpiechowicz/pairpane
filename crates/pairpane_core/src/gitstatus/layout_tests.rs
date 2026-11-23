//! The repository that is looked at is the repository gix opens: what a `.git` file, a `commondir`, a `gitdir` and a list of
//! alternates say is read as gix reads it, a link is followed hop by hop, and what gix opened is checked against what was
//! looked at.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::fixture::{Repo, pairs, within};
use super::layout::Layout;
use super::locate::is_head;
use super::testing::status;
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

    for good in ["ref: refs/heads/main\n", "ref:   refs/tags/v1\r\n", id40, &id64] {
        assert!(is_head(good.as_bytes()), "{good:?}");
    }
    // What gix's own parser of a loose reference refuses: no space after `ref:`, a name that is no reference name.
    for bad in [
        "",
        "garbage\n",
        "ref:refs/heads/x",
        "ref: refs/heads/a..b",
        "ref: refs/heads/a b",
        "ref: refs/heads/x.lock",
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
        alternates: Vec::new(),
        info_rules: 0,
    };
    fs::create_dir(repo.path("x")).expect("a folder");

    assert!(layout(".git", ".git", ".git", repo.root()).is_opened(&opened));
    // Dots are read away where they follow real folders.
    assert!(layout(".git", "x/../.git", ".git/.", &repo.path("x/..")).is_opened(&opened));
    // A place is what the system resolves it to: a link on the way makes a `..` lead elsewhere, a link to the same folder is
    // the same folder.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("x/y", repo.path("l")).expect("a link");
        std::os::unix::fs::symlink(".git", repo.path("to-git")).expect("a link");

        assert!(!layout(".git", "l/../.git", ".git", repo.root()).is_opened(&opened));
        assert!(layout(".git", "to-git", ".git", repo.root()).is_opened(&opened));
    }
    assert!(!layout(".git", "x/none/../.git", ".git", repo.root()).is_opened(&opened));

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
            // A link anywhere on the way is a repository that is not read, even one that leads nowhere.
            ("away/repo.git", false),
            ("../does-not-exist", true),
        ] {
            fs::write(sub.join(".git"), format!("gitdir: {pointer}\n")).expect("a git file");

            assert_eq!(is_repo(&sub), repo, "{pointer}");
        }
    }

    /// A link among the files of a repository itself (a deliberate trade: git follows one, and a repository whose `HEAD` or
    /// `info/exclude` is one has no status here) is a repository that is not read, whatever it leads to.
    #[test]
    fn a_link_for_info_exclude_or_attributes_or_head_is_no_repository_whatever_it_leads_to() {
        let elsewhere = tempfile::tempdir().expect("a folder");
        fs::write(elsewhere.path().join("rules"), "*.tmp\n").expect("rules");
        fs::write(elsewhere.path().join("head"), "ref: refs/heads/main\n").expect("HEAD");
        fifo(elsewhere.path(), "pipe");

        for name in [
            "info/exclude",
            "info/attributes",
            "HEAD",
            "config",
            "index",
            "packed-refs",
        ] {
            for target in [
                elsewhere.path().join("rules"),
                elsewhere.path().join("head"),
                elsewhere.path().join("missing"),
                elsewhere.path().join("pipe"),
                PathBuf::from("/dev/zero"),
                PathBuf::from("/net/host/exclude"),
            ] {
                let repo_under_test = clean(&[("a.txt", b"a")]);
                let file = repo_under_test.git_dir().join(name);
                fs::create_dir_all(file.parent().expect("a folder")).expect("a folder");
                let _ = fs::remove_file(&file);
                symlink(&target, &file).expect("a link");

                assert!(!in_time(repo_under_test.root()).repo, "{name} -> {}", target.display());
            }
        }
    }

    #[test]
    fn info_exclude_is_rules_as_a_file_and_one_over_the_size_of_a_rules_file_is_too_big() {
        let repo = clean(&[("a.txt", b"a")]);
        repo.put("x.tmp", b"x");
        fs::create_dir_all(repo.git_dir().join("info")).expect("a folder");
        fs::write(repo.git_dir().join("info/exclude"), "*.tmp\n").expect("rules");

        assert_eq!(
            pairs(&in_time(repo.root()).marks),
            vec![("x.tmp", crate::model::GitMark::Ignored)]
        );

        fs::write(repo.git_dir().join("info/exclude"), vec![b'#'; 300 * 1024]).expect("rules");
        let too_big = status(repo.root(), Limits::default(), &AtomicBool::new(false));
        assert!(matches!(too_big, Err(GitError::TooBig)), "{too_big:?}");
    }

    /// The auditor's `lexical` fixture: `gitdir: l/../real` with `l` a link to `x/y`. The system reads it as `x/real`, and gix
    /// takes the `..` out by reading the path and opens `real` beside the link, where a pipe is for `config`: what was looked
    /// at is not what is opened. A link before a `..` is a repository that is not read, and nothing waits.
    #[test]
    fn a_gitdir_that_climbs_out_of_a_link_is_no_repository_and_nothing_waits_for_the_pipe_it_hides() {
        let folder = tempfile::tempdir().expect("a folder");
        let root = gix::path::realpath(folder.path()).expect("its real path");
        let work = root.join("w");
        fs::create_dir_all(work.join("x/y")).expect("folders");
        symlink("x/y", work.join("l")).expect("a link");
        git_folder(&work.join("x/real"));
        git_folder(&work.join("real"));
        fifo(&work.join("real"), "config");

        for pointer in ["l/../real", "x/y/../real"] {
            fs::write(work.join(".git"), format!("gitdir: {pointer}\n")).expect("a git file");

            // The second is the same place with no link before its dots, `x/real`, a repository; the first is refused for its
            // link, though the system would read it as that place too.
            assert_eq!(is_repo(&work), pointer != "l/../real", "{pointer}");
        }

        // The same through a `commondir`, which names the pipe's folder.
        fs::write(work.join(".git"), "gitdir: real\n").expect("a git file");
        assert!(!is_repo(&work), "a pipe for config is no repository");
        fs::remove_file(work.join("real/config")).expect("removed");
        fs::write(work.join("real/commondir"), "../l/../x/real\n").expect("commondir");
        assert!(!is_repo(&work), "a commondir with a link before its dots");
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
