//! Ignore files: what they leave out (as ripgrep reads them) and what a hostile one cannot do to the search.

use pretty_assertions::assert_eq;

use crate::model::FindCriteria;

use super::testing::{named, put, search, temp};

fn ignoring() -> FindCriteria {
    FindCriteria {
        skip_ignored: true,
        ..named("*")
    }
}

/// The files found when the ignore files are read, without what is inside a `.git` folder.
fn files(root: &std::path::Path) -> Vec<String> {
    let found = search(root, ignoring());

    found
        .hits()
        .into_iter()
        .filter(|hit| {
            hit.kind == crate::model::EntryKind::File && !hit.path.starts_with(".git/") && !hit.path.contains("/.git/")
        })
        .map(|hit| hit.path)
        .collect()
}

/// Searches `root` with the ignore files read, checks that `found` is among the results (the search ended, whatever the
/// ignore files held) and returns how many things were skipped.
fn skipped(root: &std::path::Path, found: &str) -> u64 {
    let results = search(root, ignoring());

    assert!(results.paths().contains(&found.to_owned()), "{:?}", results.paths());

    results.done().1
}

#[test]
fn ignore_files_leave_out_files_and_folders_and_a_negation_brings_one_back() {
    let root = temp();
    put(root.path(), ".ignore", b"*.log\nbuild/\n!keep.log\n# a comment\n\n");
    put(root.path(), "a.log", b"x");
    put(root.path(), "keep.log", b"x");
    put(root.path(), "build/out.txt", b"x");
    put(root.path(), "src/build", b"a file named like the folder");
    put(root.path(), "src/main.rs", b"x");

    assert_eq!(files(root.path()), [".ignore", "keep.log", "src/build", "src/main.rs"]);
}

#[test]
fn nothing_is_left_out_unless_asked_to() {
    let root = temp();
    put(root.path(), ".ignore", b"*.log\n");
    put(root.path(), "a.log", b"x");

    let found = search(root.path(), named("*.log"));

    assert_eq!(found.paths(), ["a.log"]);
}

#[test]
fn a_deeper_file_overrides_a_shallower_one() {
    let root = temp();
    put(root.path(), ".ignore", b"*.log\n/top.txt\n");
    put(root.path(), "top.txt", b"x");
    put(root.path(), "a.log", b"x");
    put(root.path(), "sub/.ignore", b"!*.log\n*.txt\n");
    put(root.path(), "sub/b.log", b"x");
    put(root.path(), "sub/c.txt", b"x");
    put(root.path(), "sub/top.txt", b"x");
    put(root.path(), "sub/deep/d.log", b"x");

    assert_eq!(
        files(root.path()),
        [".ignore", "sub/.ignore", "sub/b.log", "sub/deep/d.log"]
    );
}

#[test]
fn a_pattern_with_a_slash_is_relative_to_the_folder_of_its_file() {
    let root = temp();
    put(root.path(), "sub/.ignore", b"/only-here.txt\nlib/gen.txt\n");
    put(root.path(), "sub/only-here.txt", b"x");
    put(root.path(), "sub/lib/gen.txt", b"x");
    put(root.path(), "sub/deeper/only-here.txt", b"x");
    put(root.path(), "only-here.txt", b"x");

    assert_eq!(
        files(root.path()),
        ["only-here.txt", "sub/.ignore", "sub/deeper/only-here.txt"]
    );
}

#[test]
fn a_gitignore_counts_inside_a_repository_only() {
    let outside = temp();
    put(outside.path(), ".gitignore", b"*.log\n");
    put(outside.path(), "a.log", b"x");

    assert_eq!(files(outside.path()), [".gitignore", "a.log"]);

    let inside = temp();
    put(inside.path(), ".git/HEAD", b"ref: refs/heads/main\n");
    put(inside.path(), ".gitignore", b"*.log\n");
    put(inside.path(), "a.log", b"x");
    put(inside.path(), "sub/b.log", b"x");

    assert_eq!(files(inside.path()), [".gitignore"]);
}

#[test]
fn a_repository_above_the_folder_searched_counts_and_one_below_it_ends_its_reach() {
    let top = temp();
    put(top.path(), ".git/HEAD", b"x");
    put(top.path(), ".gitignore", b"*.log\n/secret/\n");
    put(top.path(), "proj/a.log", b"x");
    put(top.path(), "proj/a.txt", b"x");
    put(top.path(), "proj/nested/.git/HEAD", b"x");
    put(top.path(), "proj/nested/b.log", b"x");

    let root = top.path().join("proj");

    assert_eq!(files(&root), ["a.txt", "nested/b.log"]);
}

#[test]
fn a_dot_ignore_comes_before_a_gitignore_and_a_gitignore_before_the_exclude_file() {
    let root = temp();
    put(root.path(), ".git/info/exclude", b"*.tmp\nexcluded.txt\n");
    put(root.path(), ".gitignore", b"*.txt\n!excluded.txt\n");
    put(root.path(), ".ignore", b"!ignored.txt\n");
    put(root.path(), "ignored.txt", b"x");
    put(root.path(), "plain.txt", b"x");
    put(root.path(), "excluded.txt", b"x");
    put(root.path(), "x.tmp", b"x");

    assert_eq!(
        files(root.path()),
        [".gitignore", ".ignore", "excluded.txt", "ignored.txt"]
    );
}

#[test]
fn a_folder_that_holds_a_gitdir_file_is_a_repository_and_the_file_is_not_followed() {
    let elsewhere = temp();
    put(elsewhere.path(), "info/exclude", b"*.secret\n");
    let root = temp();
    put(
        root.path(),
        ".git",
        format!("gitdir: {}\n", elsewhere.path().display()).as_bytes(),
    );
    put(root.path(), ".gitignore", b"*.log\n");
    put(root.path(), "a.log", b"x");
    put(root.path(), "b.secret", b"x");

    assert_eq!(files(root.path()), [".git", ".gitignore", "b.secret"]);
}

#[test]
fn an_ignore_file_that_is_too_big_is_left_out_whole_and_counted() {
    let root = temp();
    let patterns = "*.log\n".repeat(200_000);
    put(root.path(), ".ignore", patterns.as_bytes());
    put(root.path(), "sub/.ignore", &vec![b'x'; super::ignored::MAX_LINE + 1]);
    put(root.path(), "a.log", b"x");

    assert_eq!(skipped(root.path(), "a.log"), 2);
}

#[test]
fn an_ignore_file_with_too_many_lines_is_left_out_and_counted() {
    let root = temp();
    put(
        root.path(),
        ".ignore",
        "*.log\n".repeat(super::ignored::MAX_LINES + 1).as_bytes(),
    );
    put(root.path(), "a.log", b"x");

    assert_eq!(skipped(root.path(), "a.log"), 1);
}

#[test]
fn an_ignore_file_with_exactly_the_most_lines_and_a_final_newline_is_read() {
    let root = temp();
    put(
        root.path(),
        ".ignore",
        "*.log\n".repeat(super::ignored::MAX_LINES).as_bytes(),
    );
    put(root.path(), "a.log", b"x");
    put(root.path(), "b.txt", b"x");

    let found = search(root.path(), ignoring());

    assert_eq!(found.paths(), [".ignore", "b.txt"]);
    assert_eq!(found.done().1, 0);
}

/// The lines of the heavy ignore file, and how many folders it is linked into: more than the budget holds.
const HEAVY: usize = 1_000;
const FOLDERS: usize = 70;
const _: () = assert!(
    FOLDERS * HEAVY > super::ignored::HELD_LINES,
    "the folders must outweigh the budget"
);

/// An ignore file of `lines` distinct patterns, the first of which leaves out `*.ignored-me`.
fn heavy(lines: usize) -> String {
    let mut text = String::from("*.ignored-me\n");

    for index in 1..lines {
        text.push_str(&format!("generated-pattern-{index}-*.log\n"));
    }

    text
}

/// The rules of `root` for a search of it, to be asked about paths one by one.
fn ignores(root: &std::path::Path) -> (super::ignored::Ignores, std::sync::Arc<std::sync::atomic::AtomicU64>) {
    let skipped = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));

    (
        super::ignored::Ignores::new(root, std::sync::Arc::clone(&skipped)),
        skipped,
    )
}

#[test]
fn many_folders_with_one_heavy_ignore_file_never_hold_more_than_the_budget() {
    let root = temp();
    let text = heavy(HEAVY);
    put(root.path(), "heavy.rules", text.as_bytes());
    for index in 0..FOLDERS {
        let folder = root.path().join(format!("d{index}"));

        std::fs::create_dir_all(&folder).expect("folder");
        std::fs::hard_link(root.path().join("heavy.rules"), folder.join(".ignore")).expect("a hard link");
        put(&folder, "keep.txt", b"x");
        put(&folder, "gone.ignored-me", b"x");
    }

    let (ignores, skipped) = ignores(root.path());
    for index in 0..FOLDERS {
        let folder = root.path().join(format!("d{index}"));

        assert!(ignores.leaves_out(&folder.join("gone.ignored-me"), false));
        assert!(!ignores.leaves_out(&folder.join("keep.txt"), false));
    }

    let (lines, bytes) = ignores.peak();
    assert!(lines > 0 && lines <= super::ignored::HELD_LINES, "{lines} lines");
    assert!(bytes <= super::ignored::HELD_BYTES, "{bytes} bytes");
    assert_eq!(skipped.load(std::sync::atomic::Ordering::Relaxed), 0);

    let found = search(root.path(), ignoring());
    assert!(found.paths().contains(&"d69/keep.txt".to_owned()));
    assert!(!found.paths().contains(&"d69/gone.ignored-me".to_owned()));
}

#[test]
fn rules_that_the_folders_in_use_hold_past_the_budget_are_left_out_and_counted() {
    let root = temp();
    let text = heavy(HEAVY);
    put(root.path(), "heavy.rules", text.as_bytes());

    // Each folder is below the one before, so the rules of all of them are in use for a path at the bottom.
    let mut folder = root.path().to_owned();
    for index in 0..FOLDERS {
        folder.push(format!("n{index}"));

        std::fs::create_dir_all(&folder).expect("folder");
        std::fs::hard_link(root.path().join("heavy.rules"), folder.join(".ignore")).expect("a hard link");
    }

    let (ignores, skipped) = ignores(root.path());
    ignores.leaves_out(&folder.join("a.txt"), false);

    let (lines, _) = ignores.peak();
    let counted = skipped.load(std::sync::atomic::Ordering::Relaxed);
    assert!(lines <= super::ignored::HELD_LINES, "{lines} lines");
    assert!(counted > 0 && counted < FOLDERS as u64, "{counted} left out");
}

#[test]
fn a_file_that_cannot_be_used_is_counted_once_however_many_threads_ask_for_its_folder() {
    let root = temp();
    put(
        root.path(),
        "sub/.ignore",
        "*.log\n".repeat(super::ignored::MAX_LINES + 1).as_bytes(),
    );

    let (ignores, skipped) = ignores(root.path());
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for index in 0..50 {
                    ignores.leaves_out(&root.path().join(format!("sub/f{index}.txt")), false);
                }
            });
        }
    });

    assert_eq!(skipped.load(std::sync::atomic::Ordering::Relaxed), 1);
}

#[test]
fn a_big_file_without_a_newline_does_not_hang_or_abort_the_search_wherever_it_is() {
    let top = temp();
    let big = vec![b'a'; 2 * 1024 * 1024];
    put(top.path(), ".ignore", &big);
    put(top.path(), "proj/.ignore", &big);
    put(top.path(), "proj/sub/.ignore", &big);
    put(top.path(), "proj/sub/file.txt", b"x");

    assert_eq!(skipped(&top.path().join("proj"), "sub/file.txt"), 3);
}

#[cfg(unix)]
mod hostile {
    use std::os::unix::fs::symlink;

    use pretty_assertions::assert_eq;

    use crate::view::testing::fifo;

    use super::super::testing::folder;
    use super::*;

    fn pipes_everywhere() -> tempfile::TempDir {
        let top = temp();
        folder(top.path(), "proj/sub");
        put(top.path(), "proj/sub/file.txt", b"x");
        for place in ["", "proj", "proj/sub"] {
            let folder = top.path().join(place);
            fifo(&folder, ".ignore");
        }
        fifo(&top.path().join("proj"), ".gitignore");
        put(top.path(), "proj/.git/HEAD", b"x");

        top
    }

    #[test]
    fn a_pipe_is_never_waited_on_in_the_folder_above_in_the_folder_or_below_it() {
        let top = pipes_everywhere();

        // Three `.ignore` pipes and the `.gitignore` one.
        assert_eq!(skipped(&top.path().join("proj"), "sub/file.txt"), 4);
    }

    #[test]
    fn a_link_to_an_endless_device_is_not_read() {
        let root = temp();
        symlink("/dev/zero", root.path().join(".ignore")).expect("link");
        put(root.path(), "sub/file.txt", b"x");
        symlink("/dev/zero", root.path().join("sub/.ignore")).expect("link");

        assert_eq!(skipped(root.path(), "sub/file.txt"), 2);
    }

    #[test]
    fn a_link_to_an_ignore_file_is_not_followed() {
        let elsewhere = temp();
        put(elsewhere.path(), "rules", b"*.txt\n");
        let root = temp();
        symlink(elsewhere.path().join("rules"), root.path().join(".ignore")).expect("link");
        put(root.path(), "a.txt", b"x");

        assert_eq!(skipped(root.path(), "a.txt"), 1);
    }

    #[test]
    fn a_git_folder_that_is_a_link_does_not_lead_to_an_exclude_file() {
        let elsewhere = temp();
        put(elsewhere.path(), "info/exclude", b"*.txt\n");
        let root = temp();
        symlink(elsewhere.path(), root.path().join(".git")).expect("link");
        put(root.path(), "a.txt", b"x");

        assert_eq!(skipped(root.path(), "a.txt"), 0);
    }
}
