//! What is wrong with the rows of a rename preview: a new name that cannot be used, two rows with one new name, and a
//! new name that something in the folder has, unless the rename moves that away first.
//!
//! The last one is a cascade. A row whose new name is the name of another row that gets renamed too (a swap, a cycle, a
//! chain) is fine as long as that other row is fine. When the end of a chain is blocked, every row before it is
//! blocked as well, and so on back to its start.

use std::collections::{HashMap, HashSet};

use crate::listing::platform;
use crate::model::{RenameProblem, RenameRow};
use crate::ops::{fold_case, name_fits};

/// `rows` with a problem set on every one that has one. A row that came with a problem keeps it, one whose new name is
/// its own name is left alone, and the others are judged here. `taken` tells whether a name is in the folder.
pub(super) fn check(mut rows: Vec<RenameRow>, taken: impl Fn(&str) -> bool) -> Vec<RenameRow> {
    let mut changed = Vec::new();

    for (index, row) in rows.iter_mut().enumerate() {
        if row.problem.is_some() || row.to == row.from {
            continue;
        }

        row.problem = unusable(&row.to);
        if row.problem.is_none() {
            changed.push(index);
        }
    }

    let moving = without_duplicates(&mut rows, changed);
    block_taken(&mut rows, &moving, taken);

    rows
}

/// Why `name` cannot be the name of an item here.
fn unusable(name: &str) -> Option<RenameProblem> {
    if name.is_empty() {
        Some(RenameProblem::Empty)
    } else if !platform::is_plain_name(name) {
        Some(RenameProblem::Invalid)
    } else if !name_fits(name) {
        Some(RenameProblem::TooLong)
    } else {
        None
    }
}

/// Marks the rows of `candidates` that share a new name, and returns the others.
fn without_duplicates(rows: &mut [RenameRow], candidates: Vec<usize>) -> Vec<usize> {
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for &index in &candidates {
        groups.entry(fold_case(&rows[index].to)).or_default().push(index);
    }

    for group in groups.values().filter(|group| group.len() > 1) {
        for &index in group {
            rows[index].problem = Some(RenameProblem::Duplicate);
        }
    }

    candidates
        .into_iter()
        .filter(|&index| rows[index].problem.is_none())
        .collect()
}

/// Marks the rows of `moving` whose new name is taken and stays taken.
fn block_taken(rows: &mut [RenameRow], moving: &[usize], taken: impl Fn(&str) -> bool) {
    let leaving: HashMap<String, usize> = moving
        .iter()
        .map(|&index| (fold_case(&rows[index].from), index))
        .collect();
    let moved: HashSet<usize> = moving.iter().copied().collect();
    // What stays where it is, and is there: every row that is not moving, but an item that is gone.
    let staying: HashSet<String> = rows
        .iter()
        .enumerate()
        .filter(|(index, row)| row.problem != Some(RenameProblem::Missing) && !moved.contains(index))
        .map(|(_, row)| fold_case(&row.from))
        .collect();

    // The rows that wait for another row to move away, by that row.
    let mut waiting: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut blocked = Vec::new();

    for &index in moving {
        let target = fold_case(&rows[index].to);

        match leaving.get(&target) {
            // A change of case only, where the filesystem ignores case: the name that is taken is its own.
            Some(&holder) if holder == index => {}
            Some(&holder) => waiting.entry(holder).or_default().push(index),
            None if staying.contains(&target) || taken(&rows[index].to) => blocked.push(index),
            None => {}
        }
    }

    for &index in &blocked {
        rows[index].problem = Some(RenameProblem::Exists);
    }

    while let Some(index) = blocked.pop() {
        for dependent in waiting.remove(&index).unwrap_or_default() {
            rows[dependent].problem = Some(RenameProblem::Exists);
            blocked.push(dependent);
        }
    }
}
