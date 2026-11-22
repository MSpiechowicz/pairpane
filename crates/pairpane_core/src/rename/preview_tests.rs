//! The preview of a rule on a real folder: a row for each item, the problems the folder causes, and the fault of a rule.

use std::fs;
use std::path::Path;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::testing::{rule, templated};
use super::*;

fn folder_with(files: &[&str]) -> TempDir {
    let folder = tempfile::tempdir().expect("temp dir");
    for name in files {
        fs::write(folder.path().join(name), name).expect("file");
    }

    folder
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

/// A row of a preview as `(from, to, problem)`.
type Shown = (String, String, Option<RenameProblem>);

/// The rows of a preview, with its counts of the items that change and those with a problem.
fn rows_of(folder: &Path, list: &[&str], rule: &RenameRule) -> (Vec<Shown>, u32, u32) {
    let RenamePreview::Rows {
        rows,
        changed,
        problems,
    } = preview_in(folder, &names(list), rule)
    else {
        panic!("a rule that works");
    };
    let rows = rows.into_iter().map(|row| (row.from, row.to, row.problem)).collect();

    (rows, changed, problems)
}

fn row(from: &str, to: &str, problem: Option<RenameProblem>) -> Shown {
    (from.to_owned(), to.to_owned(), problem)
}

fn counting(start: u32) -> RenameRule {
    RenameRule {
        counter: RenameCounter {
            start,
            step: 1,
            digits: 2,
        },
        ..templated("[N]-[C]", "[E]")
    }
}

#[test]
fn every_item_gets_a_row_in_the_order_it_was_given_and_the_counter_follows_that_order() {
    let folder = folder_with(&["a.txt", "b.txt", "c"]);

    let (rows, changed, problems) = rows_of(folder.path(), &["c", "a.txt", "b.txt"], &counting(1));

    assert_eq!(
        rows,
        [
            row("c", "c-01", None),
            row("a.txt", "a-02.txt", None),
            row("b.txt", "b-03.txt", None)
        ]
    );
    assert_eq!((changed, problems), (3, 0));
}

#[test]
fn a_rule_that_changes_nothing_leaves_the_rows_unchanged_and_without_problems() {
    let folder = folder_with(&["a.txt", "b"]);

    let (rows, changed, problems) = rows_of(folder.path(), &["a.txt", "b"], &rule());

    assert_eq!(rows, [row("a.txt", "a.txt", None), row("b", "b", None)]);
    assert_eq!((changed, problems), (0, 0));
}

#[test]
fn an_item_that_is_gone_or_not_a_plain_name_is_missing() {
    let folder = folder_with(&["a.txt"]);

    let (rows, changed, problems) = rows_of(folder.path(), &["a.txt", "gone.txt", "../a.txt", ""], &counting(1));

    assert_eq!(rows[0], row("a.txt", "a-01.txt", None));
    assert_eq!(rows[1], row("gone.txt", "gone.txt", Some(RenameProblem::Missing)));
    assert_eq!(rows[2].2, Some(RenameProblem::Missing));
    assert_eq!(rows[3].2, Some(RenameProblem::Missing));
    assert_eq!((changed, problems), (1, 3));
}

#[test]
fn a_name_that_is_not_valid_text_is_unreadable() {
    let folder = folder_with(&["bad\u{fffd}.txt"]);

    let (rows, _, problems) = rows_of(folder.path(), &["bad\u{fffd}.txt"], &counting(1));

    assert_eq!(
        rows,
        [row(
            "bad\u{fffd}.txt",
            "bad\u{fffd}.txt",
            Some(RenameProblem::Unreadable)
        )]
    );
    assert_eq!(problems, 1);
}

#[test]
fn a_new_name_that_the_folder_has_is_taken_and_one_that_it_does_not_is_free() {
    let folder = folder_with(&["a.txt", "b.txt", "a-01.txt"]);

    let (rows, changed, problems) = rows_of(folder.path(), &["a.txt", "b.txt"], &counting(1));

    assert_eq!(rows[0], row("a.txt", "a-01.txt", Some(RenameProblem::Exists)));
    assert_eq!(rows[1], row("b.txt", "b-02.txt", None));
    assert_eq!((changed, problems), (1, 1));
}

#[test]
fn a_new_name_that_a_link_has_is_taken_even_when_the_link_leads_nowhere() {
    let folder = folder_with(&["a.txt"]);
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink("nowhere", folder.path().join("a-01.txt"));
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file("nowhere", folder.path().join("a-01.txt"));
    if made.is_err() {
        return;
    }

    let (rows, _, _) = rows_of(folder.path(), &["a.txt"], &counting(1));

    assert_eq!(rows[0].2, Some(RenameProblem::Exists));
}

#[test]
fn a_swap_and_a_chain_in_the_folder_are_fine_and_a_blocked_end_blocks_them_all() {
    let swap = folder_with(&["1.txt", "2.txt"]);
    let chain = folder_with(&["1.txt", "2.txt", "3.txt"]);
    let blocked = folder_with(&["1.txt", "2.txt", "3.txt", "4.txt"]);
    let numbered = templated("[C]", "[E]");

    let (swapped, _, problems) = rows_of(swap.path(), &["2.txt", "1.txt"], &numbered);
    assert_eq!(swapped, [row("2.txt", "1.txt", None), row("1.txt", "2.txt", None)]);
    assert_eq!(problems, 0);

    let (_, changed, problems) = rows_of(chain.path(), &["1.txt", "2.txt", "3.txt"], &numbered_from(2));
    assert_eq!((changed, problems), (3, 0));

    let (rows, changed, problems) = rows_of(blocked.path(), &["1.txt", "2.txt", "3.txt"], &numbered_from(2));
    assert_eq!((changed, problems), (0, 3));
    assert!(rows.iter().all(|row| row.2 == Some(RenameProblem::Exists)));
}

fn numbered_from(start: u32) -> RenameRule {
    RenameRule {
        counter: RenameCounter {
            start,
            step: 1,
            digits: 1,
        },
        ..templated("[C]", "[E]")
    }
}

#[test]
fn two_items_that_get_one_name_are_duplicates() {
    let folder = folder_with(&["a.txt", "b.txt"]);
    let same = templated("same", "[E]");

    let (rows, changed, problems) = rows_of(folder.path(), &["a.txt", "b.txt"], &same);

    assert!(
        rows.iter()
            .all(|row| row.1 == "same.txt" && row.2 == Some(RenameProblem::Duplicate))
    );
    assert_eq!((changed, problems), (0, 2));
}

#[test]
fn a_new_name_that_is_empty_invalid_or_too_long_is_a_problem() {
    let folder = folder_with(&["a.txt", "b.txt", "c.txt"]);
    let mut empty = templated("", "");
    let (rows, _, _) = rows_of(folder.path(), &["a.txt"], &empty);
    assert_eq!(rows[0].2, Some(RenameProblem::Empty));

    empty.name = "x/y".to_owned();
    let (rows, _, _) = rows_of(folder.path(), &["b.txt"], &empty);
    assert_eq!(rows[0].2, Some(RenameProblem::Invalid));

    empty.name = "n".repeat(300);
    let (rows, _, _) = rows_of(folder.path(), &["c.txt"], &empty);
    assert_eq!(rows[0].2, Some(RenameProblem::TooLong));
}

#[test]
fn a_folder_has_no_extension_and_a_link_to_one_is_a_folder() {
    let folder = folder_with(&["v1.2"]);
    fs::create_dir(folder.path().join("v2.0")).expect("folder");
    let renaming = templated("[N]-x", "[E]");

    let (rows, _, _) = rows_of(folder.path(), &["v1.2", "v2.0"], &renaming);
    assert_eq!(rows[0].1, "v1-x.2", "a file");
    assert_eq!(rows[1].1, "v2.0-x", "a folder");

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(folder.path().join("v2.0"), folder.path().join("v3.0")).expect("link");
        let (rows, _, _) = rows_of(folder.path(), &["v3.0"], &renaming);
        assert_eq!(rows[0].1, "v3.0-x", "a link to a folder");
    }
}

#[test]
fn a_change_of_case_only_is_not_taken_by_the_item_itself() {
    let folder = folder_with(&["a.txt"]);
    let mut upper = rule();
    upper.case = CaseChange::Upper;

    let (rows, changed, _) = rows_of(folder.path(), &["a.txt"], &upper);

    assert_eq!(rows, [row("a.txt", "A.TXT", None)]);
    assert_eq!(changed, 1);
}

#[test]
fn a_rule_that_cannot_be_used_is_answered_with_its_fault() {
    let folder = folder_with(&["a.txt"]);

    let RenamePreview::Rule { error } = preview_in(folder.path(), &names(&["a.txt"]), &templated("[Q]", "[E]")) else {
        panic!("a rule that does not work");
    };

    assert_eq!(error.field, RuleField::Name);
}

#[test]
fn the_time_of_the_item_gives_the_date_and_a_time_zone_that_is_known() {
    let folder = folder_with(&["a.txt"]);

    let (rows, _, problems) = rows_of(folder.path(), &["a.txt"], &templated("[YMD] [hms] [N]", "[E]"));

    assert_eq!(problems, 0);
    let to = &rows[0].1;
    assert!(
        to.ends_with(" a.txt") && to.len() == "2024-01-02 03-04-05 a.txt".len(),
        "{to}"
    );
}
