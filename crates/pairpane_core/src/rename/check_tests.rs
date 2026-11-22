//! The problems of the rows of a preview: unusable names, duplicates, names that are taken, and the cascade through
//! the names that the rename moves away.

use pretty_assertions::assert_eq;

use super::check::check;
use crate::model::{RenameProblem, RenameRow};
use crate::ops::MAX_NAME_UNITS;

fn row(from: &str, to: &str) -> RenameRow {
    RenameRow {
        from: from.to_owned(),
        to: to.to_owned(),
        problem: None,
    }
}

fn flagged(from: &str, problem: RenameProblem) -> RenameRow {
    RenameRow {
        problem: Some(problem),
        ..row(from, from)
    }
}

/// The problems of the rows made of `pairs` in a folder that has the items `present` besides them.
fn problems(pairs: &[(&str, &str)], present: &[&str]) -> Vec<Option<RenameProblem>> {
    let rows = pairs.iter().map(|(from, to)| row(from, to)).collect();

    check(rows, |name| present.contains(&name))
        .into_iter()
        .map(|row| row.problem)
        .collect()
}

const OK: Option<RenameProblem> = None;
const TAKEN: Option<RenameProblem> = Some(RenameProblem::Exists);

#[test]
fn a_row_that_changes_nothing_has_no_problem_whatever_the_folder_holds() {
    assert_eq!(problems(&[("a", "a")], &["a"]), [OK]);
}

#[test]
fn a_new_name_that_cannot_be_used_is_a_problem_of_its_kind() {
    let too_long = "n".repeat(MAX_NAME_UNITS + 1);
    let names = ["", ".", "..", "x/y", "x\0y", too_long.as_str(), "fine"];
    let verdicts: Vec<_> = names.iter().map(|name| problems(&[("a", name)], &[])[0]).collect();

    assert_eq!(
        verdicts,
        [
            Some(RenameProblem::Empty),
            Some(RenameProblem::Invalid),
            Some(RenameProblem::Invalid),
            Some(RenameProblem::Invalid),
            Some(RenameProblem::Invalid),
            Some(RenameProblem::TooLong),
            OK,
        ]
    );
}

#[cfg(windows)]
#[test]
fn a_name_that_windows_reads_as_a_device_is_invalid() {
    assert_eq!(
        problems(&[("a", "NUL.txt"), ("b", "ends.")], &[]),
        [Some(RenameProblem::Invalid); 2]
    );
}

#[test]
fn a_problem_that_a_row_came_with_is_kept() {
    let rows = vec![
        flagged("gone", RenameProblem::Missing),
        flagged("bad\u{fffd}", RenameProblem::Unreadable),
        flagged("undated", RenameProblem::NoDate),
        row("a", "b"),
    ];

    let checked: Vec<_> = check(rows, |_| false).into_iter().map(|row| row.problem).collect();

    assert_eq!(
        checked,
        [
            Some(RenameProblem::Missing),
            Some(RenameProblem::Unreadable),
            Some(RenameProblem::NoDate),
            OK
        ]
    );
}

#[test]
fn rows_with_one_new_name_are_duplicates_of_each_other_only() {
    assert_eq!(
        problems(&[("a", "x"), ("b", "x"), ("c", "y"), ("d", "z")], &[]),
        [Some(RenameProblem::Duplicate), Some(RenameProblem::Duplicate), OK, OK]
    );
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
fn new_names_that_differ_in_case_only_are_duplicates_where_the_filesystem_ignores_case() {
    assert_eq!(
        problems(&[("a", "X"), ("b", "x")], &[]),
        [Some(RenameProblem::Duplicate); 2]
    );
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[test]
fn new_names_that_differ_in_case_only_are_different_names_where_the_filesystem_tells_them_apart() {
    assert_eq!(problems(&[("a", "X"), ("b", "x")], &[]), [OK, OK]);
}

#[test]
fn a_new_name_that_something_in_the_folder_has_is_taken() {
    assert_eq!(problems(&[("a", "x"), ("b", "y")], &["x"]), [TAKEN, OK]);
}

#[test]
fn a_new_name_that_a_row_keeps_is_taken_whatever_the_folder_says() {
    assert_eq!(problems(&[("a", "b"), ("b", "b")], &[]), [TAKEN, OK]);
    assert_eq!(
        problems(&[("a", "b"), ("b", "")], &[]),
        [TAKEN, Some(RenameProblem::Empty)],
        "a row that cannot be renamed stays where it is"
    );
}

#[test]
fn the_name_of_an_item_that_is_gone_is_free() {
    let rows = vec![flagged("b", RenameProblem::Missing), row("a", "b")];

    let checked: Vec<_> = check(rows, |_| false).into_iter().map(|row| row.problem).collect();

    assert_eq!(checked, [Some(RenameProblem::Missing), OK]);
}

#[test]
fn a_swap_a_cycle_and_a_chain_are_fine() {
    assert_eq!(problems(&[("a", "b"), ("b", "a")], &["a", "b"]), [OK, OK], "swap");
    assert_eq!(
        problems(&[("a", "b"), ("b", "c"), ("c", "a")], &["a", "b", "c"]),
        [OK, OK, OK],
        "cycle"
    );
    assert_eq!(
        problems(&[("a", "b"), ("b", "c"), ("c", "d")], &["a", "b", "c"]),
        [OK, OK, OK],
        "chain"
    );
}

#[test]
fn a_blocked_end_of_a_chain_blocks_every_row_before_it() {
    assert_eq!(
        problems(&[("a", "b"), ("b", "c"), ("c", "d")], &["a", "b", "c", "d"]),
        [TAKEN, TAKEN, TAKEN]
    );
    // The order of the rows does not matter.
    assert_eq!(
        problems(&[("c", "d"), ("b", "c"), ("a", "b")], &["a", "b", "c", "d"]),
        [TAKEN, TAKEN, TAKEN]
    );
}

#[test]
fn a_row_that_cannot_move_blocks_the_rows_that_need_its_name() {
    assert_eq!(
        problems(&[("a", "b"), ("b", "c"), ("c", "")], &["a", "b", "c"]),
        [TAKEN, TAKEN, Some(RenameProblem::Empty)]
    );
}

#[test]
fn a_cycle_with_a_blocked_member_is_blocked_whole() {
    // c cannot go to a: a is also the new name of b, so both are duplicates, and the rows that wait for them are blocked.
    assert_eq!(
        problems(&[("a", "b"), ("b", "a"), ("c", "a")], &["a", "b", "c"]),
        [TAKEN, Some(RenameProblem::Duplicate), Some(RenameProblem::Duplicate)]
    );
}

#[test]
fn a_change_of_case_only_is_not_blocked_by_the_item_itself() {
    // Where names ignore case the folder has `a.txt` under the new name too.
    assert_eq!(
        problems(&[("a.txt", "A.TXT")], &["A.TXT"]),
        [if cfg!(any(target_os = "macos", target_os = "windows")) {
            OK
        } else {
            TAKEN
        }]
    );
    assert_eq!(problems(&[("a.txt", "A.TXT")], &[]), [OK]);
}
