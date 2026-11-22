//! Chains of folders, deeper than the memory of a search holds, with and without a `.git` in each: what the layers of the
//! folders cost, and that the rules above them still apply.

use pretty_assertions::assert_eq;

use super::{IN_TIME, ask_each, within};
use crate::find::ignored::{HELD_BYTES, HELD_LINES, KEPT};
use crate::find::ignored_tests::{ignores, ignoring};
use crate::find::testing::{put, search, temp};

/// What every folder of a chain holds besides its name.
#[derive(Clone, Copy)]
enum Marker {
    Nothing,
    /// An empty `.git` file, as a worktree or a submodule has one.
    File,
    /// An empty `.git` folder.
    Folder,
    /// An `.ignore` file of one rule: a layer that cannot be shared with the one above.
    Rules,
}

/// A chain of `levels` folders, made below `root` as `<chain>/d/d/d/...`, each with `marker`: the folders, from the top
/// one down.
fn chain(root: &std::path::Path, marker: Marker, number: usize, levels: usize) -> Vec<std::path::PathBuf> {
    let mut folder = root.join(format!("c{number}"));
    let mut folders = vec![folder.clone()];

    for _ in 1..levels {
        folder.push("d");
        folders.push(folder.clone());
    }
    std::fs::create_dir_all(&folder).expect("a chain");

    for folder in &folders {
        match marker {
            Marker::Nothing => {}
            Marker::File => std::fs::write(folder.join(".git"), b"").expect("a .git file"),
            Marker::Folder => std::fs::create_dir(folder.join(".git")).expect("a .git folder"),
            Marker::Rules => std::fs::write(folder.join(".ignore"), b"skip-me\n").expect("an .ignore file"),
        }
    }

    folders
}

const CHAINS_IN_USE: usize = 4;
const LEVELS_IN_USE: usize = 1_000;
const _: () = assert!(CHAINS_IN_USE * (LEVELS_IN_USE * (1024 + 2 * 2 * LEVELS_IN_USE / 2)) > HELD_BYTES);

/// The last folders of `CHAINS_IN_USE` chains of `LEVELS_IN_USE` folders (as deep as a search goes) with rules in each,
/// made below `root`: more than the budget holds in layers, once each thread holds one.
fn ends(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    (0..CHAINS_IN_USE)
        .map(|number| {
            chain(root, Marker::Rules, number, LEVELS_IN_USE)
                .pop()
                .expect("a folder")
        })
        .collect()
}

/// Chains of more folders than the memory of the search remembers, whose paths cost more than the budget holds in layers
/// when each folder has one, asked about one folder after another as a walk down them does and then searched.
fn deep_chains(marker: Marker) {
    const CHAINS: usize = 21;
    const LEVELS: usize = 400;
    const _: () = assert!(
        CHAINS * LEVELS > 2 * KEPT,
        "the folders must outnumber the bound twice over"
    );

    let top = temp();
    put(top.path(), ".ignore", b"secret\n");
    let chains: Vec<Vec<std::path::PathBuf>> = (0..CHAINS)
        .map(|number| chain(top.path(), marker, number, LEVELS))
        .collect();
    // A longer one, of a thousand folders.
    let long = chain(top.path(), marker, CHAINS, 1_000);
    for end in chains.iter().chain([&long]).filter_map(|folders| folders.last()) {
        put(end, "secret", b"x");
        put(end, "found", b"x");
    }

    let root = top.path().to_owned();
    let walk = root.clone();
    let started = std::time::Instant::now();
    let (ignores, skipped, left_out) = within(IN_TIME, move || {
        let (ignores, skipped) = ignores(&walk);
        let mut left_out = 0;

        for folders in chains.iter().chain([&long]) {
            left_out += ask_each(&ignores, folders, "secret");
        }

        (ignores, skipped, left_out)
    });

    // Every folder of every chain left `secret` out by the rule of the top folder, with no more than the bound of places
    // and of the budget, and with no marker lost: one chain at a time is in use, and it fits.
    assert_eq!(left_out, CHAINS * LEVELS + 1_000);
    assert!(ignores.remembered_peak() <= KEPT, "{}", ignores.remembered_peak());
    assert_eq!(ignores.compiles(), 1);
    assert_eq!(skipped.load(std::sync::atomic::Ordering::Relaxed), 0);

    let (lines, bytes) = ignores.peak();
    assert!(lines <= HELD_LINES, "{lines} lines");
    assert!(bytes <= HELD_BYTES, "{bytes} bytes");
    // A `.git` below another that is all a folder has adds nothing: a chain of them is one layer, not a layer for each.
    assert!(bytes < HELD_BYTES / 100, "{bytes} bytes");

    // The threads of a search hold a chain each, and a marker may be lost for lack of room: only what the rules above
    // leave out is certain.
    let found = within(IN_TIME, move || search(&root, ignoring()));
    let found_files = found.paths().iter().filter(|path| path.ends_with("/found")).count();
    assert_eq!(found_files, CHAINS + 1);
    assert!(!found.paths().iter().any(|path| path.ends_with("/secret")));
    assert!(started.elapsed() < IN_TIME, "{:?}", started.elapsed());
}

#[test]
fn deep_chains_of_folders_without_ignore_files_stay_within_the_bound_and_are_searched_fast() {
    deep_chains(Marker::Nothing);
}

#[test]
fn deep_chains_of_folders_with_an_empty_git_file_in_each_stay_within_the_bound_and_are_searched_fast() {
    deep_chains(Marker::File);
}

#[test]
fn deep_chains_of_folders_with_an_empty_git_folder_in_each_stay_within_the_bound_and_are_searched_fast() {
    deep_chains(Marker::Folder);
}

#[test]
fn chains_in_use_that_cost_more_than_the_budget_share_the_layer_above_for_the_folders_they_have_no_room_for() {
    // Four threads, each holding a chain that costs 3 MB in layers: more than the budget holds together.
    const CHAINS: usize = CHAINS_IN_USE;
    const LEVELS: usize = LEVELS_IN_USE;

    let top = temp();
    put(top.path(), ".ignore", b"secret\n");
    let ends = ends(top.path());
    for end in &ends {
        put(end, "secret", b"x");
    }

    let (ignores, skipped) = ignores(top.path());
    let skipped_now = || skipped.load(std::sync::atomic::Ordering::Relaxed);
    let ask = |ignores: &crate::find::ignored::Ignores| {
        std::thread::scope(|scope| {
            for end in &ends {
                scope.spawn(move || {
                    assert!(ignores.leaves_out(&end.join("secret"), false));
                    assert!(!ignores.leaves_out(&end.join("other"), false));
                });
            }
        });
    };

    // What is in use cannot be forgotten: what there is no room for shares the layer above it, and is counted.
    ask(&ignores);
    let first = skipped_now();
    assert!(first > 0 && first < (CHAINS * LEVELS) as u64, "{first} left out");

    let (_, bytes) = ignores.peak();
    assert!(bytes <= HELD_BYTES, "{bytes} bytes");
    assert!(ignores.remembered_peak() <= KEPT, "{}", ignores.remembered_peak());

    // The same folders asked about again are counted no more.
    ask(&ignores);
    assert_eq!(skipped_now(), first);
}

#[test]
fn nested_repositories_still_end_the_rules_above_them_while_hostile_chains_in_use_starve_the_budget() {
    const REPOS: usize = 400;

    let top = temp();
    put(top.path(), ".git/HEAD", b"x");
    put(top.path(), ".gitignore", b"*.dat\n");
    put(top.path(), "benign/top.dat", b"x");
    for number in 0..REPOS {
        put(top.path(), &format!("benign/r{number}/.git/HEAD"), b"x");
        put(top.path(), &format!("benign/r{number}/a.dat"), b"x");
    }

    // The hostile chains are held by the threads that asked about their ends, and fill the budget between them.
    let hostile = top.path().join("hostile");
    let ends = ends(&hostile);

    let (ignores, _) = ignores(top.path());
    std::thread::scope(|scope| {
        for end in &ends {
            scope.spawn(|| ignores.leaves_out(&end.join("a.txt"), false));
        }
    });

    // With no room for a layer, a nested repository is still where the rules above end: its files are not left out by them.
    for number in 0..REPOS {
        let file = top.path().join(format!("benign/r{number}/a.dat"));

        assert!(!ignores.leaves_out(&file, false), "r{number}");
    }
    assert!(ignores.leaves_out(&top.path().join("benign/top.dat"), false));
    assert!(ignores.refused() > 0, "the budget was never starved");

    let (_, bytes) = ignores.peak();
    assert!(bytes <= HELD_BYTES, "{bytes} bytes");
    assert!(ignores.remembered_peak() <= KEPT, "{}", ignores.remembered_peak());

    let found = within(IN_TIME, move || search(top.path(), ignoring()));
    let dats = found.paths().iter().filter(|path| path.ends_with("/a.dat")).count();
    assert_eq!(dats, REPOS);
}
