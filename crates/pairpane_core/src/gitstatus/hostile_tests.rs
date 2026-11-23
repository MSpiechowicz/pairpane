//! Repositories made to hurt: trees that expand into billions of paths, files that are pipes or devices where gix would
//! read a file, pointers to network places, an index too big to decode, a format that is not read. Each of them has to
//! be answered quickly and with no path in the answer, and a place that is not local is never touched.

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use super::fixture::{Repo, within};
use super::testing::status;
use super::tests::clean;
use super::*;
use crate::model::GitMark::Staged as Added;

/// A repository whose `HEAD` is a tree of `levels` levels of `width` entries that are all the same subtree: `width.pow(levels)`
/// paths in a few kilobytes of objects, with the folders of the first of each level in the work tree.
fn bomb(width: usize, levels: usize) -> Repo {
    let repo = Repo::new();
    let mut tree = repo.leaf("f.txt");
    for _ in 0..levels {
        tree = repo.repeating(tree, width);
    }
    repo.commit_tree(tree);
    repo.write_index(&[]);

    repo
}

fn read(folder: &Path, limits: Limits) -> Result<GitStatus, GitError> {
    status(folder, limits, &AtomicBool::new(false))
}

#[test]
fn a_tree_that_expands_into_billions_of_paths_is_too_big_after_the_cap_and_quickly() {
    // 200 entries a level, 3 levels, all pointing at the same subtree: 8,000,000 files.
    let repo = bomb(200, 3);
    let root = repo.root().to_path_buf();
    let started = Instant::now();

    let small = within(60, {
        let root = root.clone();
        move || {
            read(
                &root,
                Limits {
                    entries: 1_000,
                    ..Limits::default()
                },
            )
        }
    });
    let small_elapsed = started.elapsed();
    // A bigger cap is reached too, and it is the cap and not the time that ends it (the time is long here: this is a build
    // without optimisations on a machine that may be busy).
    let bigger = within(120, move || {
        read(
            &root,
            Limits {
                entries: 20_000,
                time: Duration::from_secs(50),
                ..Limits::default()
            },
        )
    });

    assert!(matches!(small, Err(GitError::TooBig)), "{small:?}");
    assert!(matches!(bigger, Err(GitError::TooBig)), "{bigger:?}");
    assert!(small_elapsed < Duration::from_secs(10), "{small_elapsed:?}");
}

#[test]
fn a_folder_inside_a_tree_bomb_is_read_from_its_own_subtree_only() {
    let repo = bomb(200, 3);
    // `e000/e001` is the second level from the bottom: 200 entries, each a folder of one file.
    fs::create_dir_all(repo.path("e000/e001")).expect("folders");
    let folder = repo.path("e000/e001");

    let answer = within(20, move || read(&folder, Limits::default())).expect("a status");

    assert!(answer.repo);
    assert_eq!(answer.marks.len(), 200);
    assert!(
        answer.marks.iter().all(|entry| entry.mark == Added),
        "{:?}",
        answer.marks
    );
}

#[test]
fn trees_nested_deeper_than_the_walk_goes_are_too_big() {
    let repo = Repo::new();
    let mut tree = repo.leaf("f.txt");
    for _ in 0..400 {
        tree = repo.repeating(tree, 1);
    }
    repo.commit_tree(tree);
    repo.write_index(&[]);
    let root = repo.root().to_path_buf();

    let answer = within(20, move || read(&root, Limits::default()));

    assert!(matches!(answer, Err(GitError::TooBig)), "{answer:?}");
}

#[test]
fn a_sha256_repository_or_one_of_a_format_that_is_not_read_is_not_a_repo_and_not_an_error() {
    for config in [
        "[core]\n\trepositoryformatversion = 1\n[extensions]\n\tobjectformat = sha256\n",
        "[core]\n\trepositoryformatversion = 7\n",
    ] {
        let repo = clean(&[("a.txt", b"a")]);
        repo.put("b.txt", b"b");
        fs::write(repo.git_dir().join("config"), config).expect("a config");

        let answer = read(repo.root(), Limits::default());

        assert_eq!(
            answer.as_ref().ok(),
            Some(&GitStatus {
                repo: false,
                marks: vec![]
            }),
            "{config:?}: {answer:?}"
        );
    }
}

#[test]
fn an_index_over_its_cap_is_too_big_without_being_read() {
    let repo = clean(&[("a.txt", b"a")]);
    let index = fs::File::options()
        .write(true)
        .open(repo.index_path())
        .expect("the index");
    // A sparse file: it takes no room.
    index.set_len(super::inspect::INDEX_BYTES + 1).expect("a length");
    let root = repo.root().to_path_buf();

    let answer = within(10, move || read(&root, Limits::default()));

    assert!(matches!(answer, Err(GitError::TooBig)), "{answer:?}");
}

#[test]
fn a_damaged_index_is_an_unreadable_repository_with_no_path_in_its_text() {
    let repo = clean(&[("a.txt", b"a")]);
    fs::write(repo.index_path(), b"DIRC garbage that is no index").expect("a damaged index");

    let answer = read(repo.root(), Limits::default());

    let Err(GitError::Repository) = answer else {
        panic!("{answer:?}");
    };
}

/// Pointers that lead to a place that is local on the system of the test, spelled so that they must be refused anyway: a
/// leading `//` is how a network path starts. With one slash they are read, so a refusal is the guard's and not a mistake
/// of the repository.
#[cfg(unix)]
mod pointers {
    use super::*;

    /// `path` as a pointer with two slashes at its start.
    fn doubled(path: &Path) -> String {
        format!("/{}", path.display())
    }

    #[test]
    fn a_git_file_that_points_to_a_network_path_is_not_followed() {
        let main = clean(&[("a.txt", b"a")]);
        main.put("a.txt", b"changed");
        let folder = tempfile::tempdir().expect("a folder");
        fs::write(folder.path().join("b.txt"), b"b").expect("a file");

        for (pointer, repo) in [
            (main.git_dir().display().to_string(), true),
            (doubled(main.git_dir()), false),
            (format!("\\\\host\\share\\{}", main.git_dir().display()), false),
            (
                format!(
                    "{}/../{}/.git",
                    folder.path().display(),
                    main.root().file_name().expect("a name").to_string_lossy()
                ),
                true,
            ),
        ] {
            fs::write(folder.path().join(".git"), format!("gitdir: {pointer}\n")).expect("the git file");

            let answer = read(folder.path(), Limits::default()).expect("a status");

            assert_eq!(answer.repo, repo, "{pointer}");
        }
    }

    #[test]
    fn an_alternates_list_with_a_network_path_makes_the_folder_no_repository() {
        let repo = clean(&[("a.txt", b"a")]);
        let other = clean(&[("o.txt", b"o")]);
        fs::create_dir_all(repo.git_dir().join("objects/info")).expect("a folder");
        let list = repo.git_dir().join("objects/info/alternates");

        for (line, expected) in [
            (other.git_dir().join("objects").display().to_string(), true),
            (doubled(&other.git_dir().join("objects")), false),
            ("\\\\host\\share\\objects".to_owned(), false),
            ("//host/share/objects".to_owned(), false),
        ] {
            fs::write(&list, format!("# comment\n{line}\n")).expect("a list");

            assert_eq!(
                read(repo.root(), Limits::default()).expect("a status").repo,
                expected,
                "{line}"
            );
        }
    }

    #[test]
    fn a_commondir_with_a_network_path_makes_the_folder_no_repository() {
        let main = clean(&[("a.txt", b"a")]);
        let (linked, admin) = main.linked_worktree("linked");
        fs::write(linked.join("a.txt"), b"a").expect("a");

        for (text, expected) in [
            ("../..\n".to_owned(), true),
            (doubled(main.git_dir()), false),
            ("\\\\host\\share\\.git".to_owned(), false),
        ] {
            fs::write(admin.join("commondir"), text.as_bytes()).expect("commondir");

            assert_eq!(
                read(&linked, Limits::default()).expect("a status").repo,
                expected,
                "{text}"
            );
        }
    }
}

/// Named pipes and devices where gix reads a file (Unix): each answer must come, and it is `repo: false`.
#[cfg(unix)]
mod pipes {
    use super::*;
    use crate::view::testing::fifo;

    /// The repository `repo` read on a thread of its own, which has to answer.
    fn answer(repo: &Repo) -> Result<GitStatus, GitError> {
        let root = repo.root().to_path_buf();

        within(10, move || read(&root, Limits::default()))
    }

    fn assert_not_a_repository(repo: &Repo, what: &str) {
        let answer = answer(repo);

        assert_eq!(
            answer.as_ref().ok(),
            Some(&GitStatus {
                repo: false,
                marks: vec![]
            }),
            "{what}: {answer:?}"
        );
    }

    /// A named pipe at the place of the file `relative` of the repository's folder.
    fn pipe_at(repo: &Repo, relative: &str) {
        let path = repo.git_dir().join(relative);
        let _ = fs::remove_file(&path);
        fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
        fifo(
            path.parent().expect("a parent"),
            path.file_name().expect("a name").to_str().expect("text"),
        );
    }

    #[test]
    fn a_named_pipe_in_the_place_of_a_file_gix_reads_is_no_repository_and_nothing_waits_for_it() {
        for name in [
            "HEAD",
            "config",
            "config.worktree",
            "commondir",
            "gitdir",
            "index",
            "packed-refs",
            "info/exclude",
            "info/attributes",
            "objects/info/alternates",
            "sharedindex.0123456789012345678901234567890123456789",
        ] {
            let repo = clean(&[("a.txt", b"a")]);
            pipe_at(&repo, name);

            assert_not_a_repository(&repo, name);
        }
    }

    #[test]
    fn a_named_pipe_for_the_ref_head_leads_to_is_no_repository_through_a_chain_of_refs_too() {
        let repo = clean(&[("a.txt", b"a")]);
        let head = fs::read_to_string(repo.git_dir().join("HEAD")).expect("HEAD");
        let branch = head.trim().strip_prefix("ref: ").expect("a symbolic ref").to_owned();
        // HEAD -> refs/heads/alias -> the branch, whose file is a pipe.
        fs::write(repo.git_dir().join("refs/heads/alias"), format!("ref: {branch}\n")).expect("a ref");
        fs::write(repo.git_dir().join("HEAD"), "ref: refs/heads/alias\n").expect("HEAD");
        pipe_at(&repo, &branch);

        assert_not_a_repository(&repo, "a ref at the end of a chain");
    }

    #[test]
    fn a_link_to_dev_zero_in_the_place_of_head_or_a_ref_or_the_config_is_no_repository() {
        for relative in ["HEAD", "config", "index", "packed-refs", "info/exclude"] {
            let repo = clean(&[("a.txt", b"a")]);
            let path = repo.git_dir().join(relative);
            let _ = fs::remove_file(&path);
            fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
            std::os::unix::fs::symlink("/dev/zero", &path).expect("a link");

            assert_not_a_repository(&repo, relative);
        }
    }

    #[test]
    fn a_link_for_a_folder_of_the_refs_is_no_repository() {
        let repo = clean(&[("a.txt", b"a")]);
        let elsewhere = tempfile::tempdir().expect("a folder");
        fifo(elsewhere.path(), "main");
        fs::remove_dir_all(repo.git_dir().join("refs/heads")).expect("removed");
        std::os::unix::fs::symlink(elsewhere.path(), repo.git_dir().join("refs/heads")).expect("a link");
        fs::write(repo.git_dir().join("HEAD"), "ref: refs/heads/main\n").expect("HEAD");

        assert_not_a_repository(&repo, "a link for refs/heads");
    }

    #[test]
    fn a_pack_folder_with_a_pipe_in_it_is_no_repository() {
        let repo = clean(&[("a.txt", b"a")]);
        fs::create_dir_all(repo.git_dir().join("objects/pack")).expect("a folder");
        fifo(&repo.git_dir().join("objects/pack"), "pack-0.idx");

        assert_not_a_repository(&repo, "a pipe in objects/pack");
    }

    #[test]
    fn a_status_still_answers_for_a_repository_with_a_symbolic_link_for_the_dot_git_folder() {
        let repo = clean(&[("a.txt", b"a")]);
        repo.put("b.txt", b"untracked");
        let elsewhere = tempfile::tempdir().expect("a folder");
        let moved = elsewhere.path().join("moved.git");
        fs::rename(repo.git_dir(), &moved).expect("moved");
        std::os::unix::fs::symlink(&moved, repo.path(".git")).expect("a link");

        let answer = answer(&repo).expect("a status");

        assert!(answer.repo);
        assert_eq!(
            super::super::fixture::pairs(&answer.marks),
            vec![("b.txt", crate::model::GitMark::Untracked)]
        );
    }
}
