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
        }
    }

    folders
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
    if !matches!(marker, Marker::Nothing) {
        // The layers are what fills the budget: there are more of them than it holds.
        assert!(bytes > HELD_BYTES / 2, "{bytes} bytes");
    }

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
    // Four threads, each holding a chain of a thousand folders (as deep as a search goes) that costs 3 MB in layers: more
    // than the budget holds together.
    const CHAINS: usize = 4;
    const LEVELS: usize = 1_000;
    const _: () = assert!(CHAINS * (LEVELS * (1024 + 2 * 2 * LEVELS / 2)) > HELD_BYTES);

    let top = temp();
    put(top.path(), ".ignore", b"secret\n");
    let ends: Vec<std::path::PathBuf> = (0..CHAINS)
        .map(|number| chain(top.path(), Marker::File, number, LEVELS).pop().expect("a folder"))
        .collect();
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
