//! What the rules of ignore files hold of the memory of a search: the budget, what is forgotten for room and what never is.

use pretty_assertions::assert_eq;

use super::ignored_tests::{ignores, ignoring};
use super::testing::{put, search, temp};

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

/// The folder `name` below `root`, made, with a hard link to the heavy rules of `root` as its ignore file.
fn link_rules(root: &std::path::Path, name: String) -> std::path::PathBuf {
    let folder = root.join(name);

    link_rules_into(root, &folder);

    folder
}

fn link_rules_into(root: &std::path::Path, folder: &std::path::Path) {
    std::fs::create_dir_all(folder).expect("folder");
    std::fs::hard_link(root.join("heavy.rules"), folder.join(".ignore")).expect("a hard link");
}

#[test]
fn many_folders_with_one_heavy_ignore_file_never_hold_more_than_the_budget() {
    let root = temp();
    let text = heavy(HEAVY);
    put(root.path(), "heavy.rules", text.as_bytes());
    for index in 0..FOLDERS {
        let folder = link_rules(root.path(), format!("d{index}"));
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

        link_rules_into(root.path(), &folder);
    }

    let (ignores, skipped) = ignores(root.path());
    ignores.leaves_out(&folder.join("a.txt"), false);

    let (lines, _) = ignores.peak();
    let counted = skipped.load(std::sync::atomic::Ordering::Relaxed);
    assert!(lines <= super::ignored::HELD_LINES, "{lines} lines");
    assert!(counted > 0 && counted < FOLDERS as u64, "{counted} left out");
}

/// The levels of a chain of folders, each with one ignore file of the most lines: together they are all the budget holds.
const LEVELS: usize = 25;
const _: () = assert!(
    LEVELS * super::ignored::MAX_LINES == super::ignored::HELD_LINES,
    "the chain must fill the budget exactly"
);

/// How many folders are below the end of a chain.
const LEAVES: usize = 200;

/// The shape that made the budget thrash: a chain of `levels` folders whose ignore files (hard links to one file of the
/// most lines, but the first, which also leaves out the files named `g` when `top_rule`) fill the budget, and `LEAVES`
/// folders below the end of it, each with `leaf_lines` lines of rules, one file named `g` and one named `gone.ignored-me`.
/// Returns the folder to search.
fn chain(top: &std::path::Path, levels: usize, leaf_lines: usize, top_rule: bool) -> std::path::PathBuf {
    let root = top.join("r");
    let rules = heavy(super::ignored::MAX_LINES);
    put(top, "chain.rules", rules.as_bytes());

    let mut folder = root.clone();
    for level in 1..=levels {
        folder.push(format!("a{level}"));

        std::fs::create_dir_all(&folder).expect("folder");
        if level == 1 && top_rule {
            let text = format!(
                "g\n{}",
                rules
                    .lines()
                    .skip(1)
                    .map(|line| format!("{line}\n"))
                    .collect::<String>()
            );
            put(&folder, ".ignore", text.as_bytes());
        } else {
            std::fs::hard_link(top.join("chain.rules"), folder.join(".ignore")).expect("a hard link");
        }
    }

    let leaf = heavy(leaf_lines);
    for index in 0..LEAVES {
        let leaves = folder.join(format!("s{index}"));

        put(&leaves, ".ignore", leaf.as_bytes());
        put(&leaves, "g", b"x");
        put(&leaves, "gone.ignored-me", b"x");
    }

    root
}

/// Every path below `folder` (below it, names joined with `/`) but those of the entries named as one of `left_out`.
fn tree(folder: &std::path::Path, left_out: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    let mut todo = vec![folder.to_owned()];

    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).expect("a folder") {
            let entry = entry.expect("an entry");
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();

            if path.is_dir() {
                todo.push(path.clone());
            }
            if !left_out.contains(&name.as_str()) {
                found.push(
                    path.strip_prefix(folder)
                        .expect("below")
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    found.sort();

    found
}

/// Runs `body` on its own thread and fails the test when it takes longer than `limit`: a search that thrashes must not
/// hold up the test run.
fn within<T: Send + 'static>(limit: std::time::Duration, body: impl FnOnce() -> T + Send + 'static) -> T {
    let (sender, receiver) = std::sync::mpsc::channel();

    std::thread::spawn(move || sender.send(body()).ok());

    receiver.recv_timeout(limit).expect("done in time")
}

/// The rules asked about every entry below `root`, by eight threads, each taking whole folders as a walk does.
fn ask_all(ignores: &super::ignored::Ignores, root: &std::path::Path) {
    let folders: Vec<std::path::PathBuf> = tree(root, &[])
        .into_iter()
        .map(|path| root.join(path))
        .filter(|path| path.is_dir())
        .collect();
    let next = std::sync::atomic::AtomicUsize::new(0);

    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(folder) = folders.get(index) else {
                        break;
                    };

                    for entry in std::fs::read_dir(folder).expect("a folder") {
                        let path = entry.expect("an entry").path();

                        ignores.leaves_out(&path, path.is_dir());
                    }
                }
            });
        }
    });
}

const IN_TIME: std::time::Duration = std::time::Duration::from_secs(60);

#[test]
fn a_chain_of_big_ignore_files_filling_the_budget_is_read_once_and_the_files_below_it_are_the_ones_left_out() {
    let top = temp();
    let root = chain(top.path(), LEVELS, 1, false);
    let expected = tree(&root, &["gone.ignored-me"]);
    let started = std::time::Instant::now();

    let (ignores, skipped) = ignores(&root);
    let walk = root.clone();
    let (ignores, skipped) = within(IN_TIME, move || {
        ask_all(&ignores, &walk);

        (ignores, skipped)
    });

    assert!(started.elapsed() < IN_TIME, "{:?}", started.elapsed());

    // Each file of the chain is compiled once; the one-line files below do not fit and are not compiled at all.
    assert_eq!(ignores.compiles(), LEVELS as u64);
    assert_eq!(skipped.load(std::sync::atomic::Ordering::Relaxed), LEAVES as u64);

    let (lines, bytes) = ignores.peak();
    assert!(lines <= super::ignored::HELD_LINES, "{lines} lines");
    assert!(bytes <= super::ignored::HELD_BYTES, "{bytes} bytes");

    let found = within(IN_TIME, move || search(&root, ignoring()));
    assert_eq!(found.paths(), expected);
    assert_eq!(found.done().1, LEAVES as u64);
}

#[test]
fn a_rule_at_the_top_of_a_chain_that_fills_the_budget_applies_to_every_folder_below() {
    let top = temp();
    let root = chain(top.path(), LEVELS, 1, true);
    let (ignores, skipped) = ignores(&root);
    let end = (1..=LEVELS).fold(root.clone(), |folder, level| folder.join(format!("a{level}")));

    for index in 0..LEAVES {
        assert!(
            ignores.leaves_out(&end.join(format!("s{index}/g")), false),
            "s{index}/g"
        );
        assert!(!ignores.leaves_out(&end.join(format!("s{index}/other")), false));
    }
    assert_eq!(skipped.load(std::sync::atomic::Ordering::Relaxed), LEAVES as u64);

    let found = within(IN_TIME, move || search(&root, ignoring()));

    // The files named `g` are the 200 left out: the rule of the top folder reaches them in every folder.
    assert_eq!(found.paths(), tree(&top.path().join("r"), &["g", "gone.ignored-me"]));
    assert_eq!(found.done().0, 450);
}

#[test]
fn a_chain_that_leaves_some_room_forgets_the_folders_below_it_that_nothing_uses_and_reads_the_chain_once() {
    let top = temp();
    let root = chain(top.path(), LEVELS - 1, 20, false);
    let (ignores, skipped) = ignores(&root);
    let walk = root.clone();

    let (ignores, skipped) = within(IN_TIME, move || {
        ask_all(&ignores, &walk);

        (ignores, skipped)
    });

    // The chain is read once; the folders below it (each read when its turn comes) a few times at most, none left out.
    let compiles = ignores.compiles();
    assert!(compiles >= (LEVELS - 1 + LEAVES) as u64, "{compiles} compiled");
    assert!(compiles <= 2 * (LEVELS - 1 + LEAVES) as u64, "{compiles} compiled");
    assert_eq!(skipped.load(std::sync::atomic::Ordering::Relaxed), 0);

    let (lines, _) = ignores.peak();
    assert!(lines <= super::ignored::HELD_LINES, "{lines} lines");

    let found = within(IN_TIME, move || search(&root, ignoring()));
    assert!(!found.paths().iter().any(|path| path.ends_with("gone.ignored-me")));
    assert_eq!(found.done().1, 0);
}

#[test]
fn a_file_that_does_not_fit_forgets_only_what_it_takes_and_never_the_whole_memory() {
    let root = temp();
    let text = heavy(HEAVY);
    put(root.path(), "heavy.rules", text.as_bytes());

    // 49 000 of the 50 000 lines, in folders that nothing uses once they have been asked about.
    for index in 0..49 {
        link_rules(root.path(), format!("d{index}"));
    }
    put(root.path(), "big/.ignore", heavy(2 * HEAVY).as_bytes());

    let (ignores, skipped) = ignores(root.path());
    for index in 0..49 {
        ignores.leaves_out(&root.path().join(format!("d{index}/a.txt")), false);
    }
    assert_eq!(ignores.compiles(), 49);

    // One more folder needs 1 000 lines more than there is room for: the one asked about longest ago goes, nothing else.
    ignores.leaves_out(&root.path().join("big/a.txt"), false);
    assert_eq!(ignores.compiles(), 50);

    for index in 1..49 {
        assert!(ignores.leaves_out(&root.path().join(format!("d{index}/a.ignored-me")), false));
    }
    assert_eq!(ignores.compiles(), 50);
    assert_eq!(skipped.load(std::sync::atomic::Ordering::Relaxed), 0);
}

/// Folders without ignore files: how many are asked about, more than the map of remembered folders holds.
const PLAIN: usize = super::ignored::KEPT + 900;

/// Asks about `name` in each of `folders`, one at a time, as a walk down them does.
fn ask_each(ignores: &super::ignored::Ignores, folders: &[std::path::PathBuf], name: &str) -> usize {
    folders
        .iter()
        .filter(|folder| ignores.leaves_out(&folder.join(name), false))
        .count()
}

#[test]
fn folders_without_ignore_files_are_remembered_within_the_bound_and_the_rules_above_still_apply() {
    let root = temp();
    put(root.path(), ".ignore", b"secret\n");
    let folders: Vec<std::path::PathBuf> = (0..PLAIN).map(|index| root.path().join(format!("f{index}"))).collect();
    for folder in &folders {
        std::fs::create_dir(folder).expect("folder");
    }

    let (ignores, skipped) = ignores(root.path());

    assert_eq!(ask_each(&ignores, &folders, "secret"), PLAIN);
    assert_eq!(ask_each(&ignores, &folders, "other"), 0);
    assert!(
        ignores.remembered_peak() <= super::ignored::KEPT,
        "{}",
        ignores.remembered_peak()
    );
    assert_eq!(ignores.compiles(), 1);
    assert_eq!(skipped.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[test]
fn folders_without_rules_share_those_above_them_and_the_ones_with_rules_below_them_still_apply() {
    let top = temp();
    let root = top.path().join("proj");
    put(&root, ".git/HEAD", b"x");
    put(&root, ".gitignore", b"*.log\n");
    put(&root, ".ignore", b"root.skip\n");
    put(&root, "a/b/c/e/.ignore", b"deep.skip\n");
    put(&root, "a/b/c/e/f/g/x", b"x");
    // A repository below the folders without rules, with no ignore file of its own, ends the reach of the `.gitignore`.
    put(&root, "a/b/c/n/.git/HEAD", b"x");
    put(&root, "a/b/c/n/sub/x", b"x");
    let c = root.join("a/b/c");

    let (ignores, skipped) = ignores(&root);
    let left_out = |path: &str| ignores.leaves_out(&c.join(path), false);

    assert!(left_out("root.skip"));
    assert!(left_out("z.log"));
    assert!(left_out("e/deep.skip"));
    assert!(left_out("e/f/g/deep.skip"));
    assert!(left_out("e/f/g/root.skip"));
    assert!(!left_out("deep.skip"));
    assert!(!left_out("e/f/g/other"));

    assert!(left_out("n/root.skip"));
    assert!(!left_out("n/y.log"));
    assert!(!left_out("n/sub/y.log"));
    assert!(left_out("n/sub/root.skip"));
    assert_eq!(skipped.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[test]
fn rules_above_folders_without_rules_are_read_once_however_many_of_those_are_remembered_and_forgotten() {
    let root = temp();
    put(root.path(), "r/.ignore", b"secret\n");
    let folders: Vec<std::path::PathBuf> = (0..PLAIN)
        .map(|index| root.path().join(format!("r/f{index}")))
        .collect();
    for folder in &folders {
        std::fs::create_dir(folder).expect("folder");
    }
    std::fs::create_dir(root.path().join("r/last")).expect("folder");

    let (ignores, _) = ignores(root.path());
    assert_eq!(ask_each(&ignores, &folders, "secret"), PLAIN);

    // The rules were remembered through all the places made and forgotten, and are still the ones asked.
    assert!(ignores.leaves_out(&root.path().join("r/last/secret"), false));
    assert_eq!(ignores.compiles(), 1);
}

// Chains of folders deeper than macOS (1 024 bytes of path) or Windows (260) allow, made and searched on Linux only.
#[cfg(target_os = "linux")]
#[path = "ignored_deep_tests.rs"]
mod deep;
