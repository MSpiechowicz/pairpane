//! Names, sizes, dates and attributes, and what `check` and `compile` refuse.

use std::io::ErrorKind;

use pretty_assertions::assert_eq;

use crate::model::{FindCriteria, FindQuery, Tri};
use crate::view::is_refusal;

use super::content_tests::text;
use super::testing::{named, put, search, temp};
use super::{MAX_MASKS, MAX_PATTERN_BYTES, check, compile};

fn refused(criteria: &FindCriteria) -> String {
    let error = check(criteria).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert!(is_refusal(&error));

    error.to_string()
}

fn compiled(criteria: FindCriteria) -> std::io::Result<super::Compiled> {
    let root = if cfg!(windows) { "C:\\data" } else { "/data" };

    compile(&FindQuery {
        root: root.to_owned(),
        criteria,
    })
}

#[test]
fn sizes_are_inclusive_and_leave_out_folders() {
    let root = temp();
    put(root.path(), "small", &[0; 10]);
    put(root.path(), "middle", &[0; 20]);
    put(root.path(), "big", &[0; 30]);
    put(root.path(), "folder/x", &[0; 1]);
    let mut criteria = named("*");

    criteria.min_size = Some(20);
    assert_eq!(search(root.path(), criteria.clone()).paths(), ["big", "middle"]);

    criteria.max_size = Some(20);
    assert_eq!(search(root.path(), criteria.clone()).paths(), ["middle"]);

    criteria.min_size = None;
    criteria.max_size = Some(0);
    assert_eq!(search(root.path(), criteria).paths(), Vec::<String>::new());
}

#[test]
fn dates_are_inclusive_in_milliseconds() {
    let root = temp();
    put(root.path(), "file", b"x");
    let modified = search(root.path(), named("file")).hits()[0].modified.expect("a time");
    let mut criteria = named("file");

    criteria.modified_from = Some(modified);
    criteria.modified_to = Some(modified);
    assert_eq!(search(root.path(), criteria.clone()).paths(), ["file"]);

    criteria.modified_from = Some(modified + 1);
    criteria.modified_to = None;
    assert!(search(root.path(), criteria.clone()).paths().is_empty());

    criteria.modified_from = None;
    criteria.modified_to = Some(modified - 1);
    assert!(search(root.path(), criteria).paths().is_empty());
}

#[test]
#[allow(
    clippy::permissions_set_readonly_false,
    reason = "the file is made writable again so its folder can be removed on Windows"
)]
fn the_read_only_attribute_is_yes_no_or_any() {
    let root = temp();
    put(root.path(), "locked", b"x");
    put(root.path(), "open", b"x");
    let mut permissions = std::fs::metadata(root.path().join("locked"))
        .expect("metadata")
        .permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(root.path().join("locked"), permissions).expect("read-only");
    let mut criteria = named("*");

    criteria.readonly = Tri::Yes;
    assert_eq!(search(root.path(), criteria.clone()).paths(), ["locked"]);

    criteria.readonly = Tri::No;
    assert_eq!(search(root.path(), criteria.clone()).paths(), ["open"]);

    criteria.readonly = Tri::Any;
    assert_eq!(search(root.path(), criteria).paths(), ["locked", "open"]);

    // Let the temporary folder be removed on Windows.
    let mut permissions = std::fs::metadata(root.path().join("locked"))
        .expect("metadata")
        .permissions();
    permissions.set_readonly(false);
    std::fs::set_permissions(root.path().join("locked"), permissions).expect("writable");
}

#[cfg(unix)]
#[test]
fn the_executable_attribute_is_a_regular_file_with_an_execute_bit() {
    use std::os::unix::fs::PermissionsExt;

    let root = temp();
    put(root.path(), "tool", b"x");
    put(root.path(), "note", b"x");
    put(root.path(), "folder/inside", b"x");
    std::fs::set_permissions(root.path().join("tool"), std::fs::Permissions::from_mode(0o755)).expect("mode");
    let mut criteria = named("tool;note;folder");

    criteria.executable = Tri::Yes;
    assert_eq!(search(root.path(), criteria.clone()).paths(), ["tool"]);

    criteria.executable = Tri::No;
    assert_eq!(search(root.path(), criteria).paths(), ["folder", "note"]);
}

#[test]
fn criteria_that_cannot_hold_are_refused_with_fixed_texts() {
    let mut criteria = named("*");
    assert_eq!(check(&criteria).ok(), Some(()));

    criteria.min_size = Some(5);
    criteria.max_size = Some(4);
    assert_eq!(refused(&criteria), "the smallest size is above the largest");

    criteria = FindCriteria {
        modified_from: Some(2),
        modified_to: Some(1),
        ..FindCriteria::default()
    };
    assert_eq!(refused(&criteria), "the first date is after the last");

    criteria = FindCriteria {
        archives: true,
        duplicates: true,
        ..FindCriteria::default()
    };
    assert_eq!(refused(&criteria), "archives are searched by name only");
    criteria.duplicates = false;
    criteria.text = Some(text("x"));
    assert_eq!(refused(&criteria), "archives are searched by name only");

    criteria = FindCriteria {
        skip_hidden: true,
        hidden: Tri::Yes,
        ..FindCriteria::default()
    };
    assert_eq!(refused(&criteria), "hidden files are skipped and asked for at once");
}

#[test]
fn the_executable_attribute_is_refused_where_it_cannot_be_told() {
    let criteria = FindCriteria {
        executable: Tri::Yes,
        ..FindCriteria::default()
    };

    assert_eq!(check(&criteria).is_err(), cfg!(windows));
}

#[test]
fn text_and_encoding_are_checked() {
    let mut found = text("x");
    found.encoding = Some("no-such-encoding".to_owned());
    let mut criteria = FindCriteria {
        text: Some(found),
        ..FindCriteria::default()
    };
    assert_eq!(refused(&criteria), "the encoding is not known");

    criteria.text = Some(text(""));
    assert_eq!(refused(&criteria), "the text is empty");

    criteria.text = Some(text("two\nlines"));
    assert_eq!(refused(&criteria), "the text must be on one line");

    criteria.text = Some(text(&"x".repeat(MAX_PATTERN_BYTES + 1)));
    assert_eq!(refused(&criteria), "the text is too long");

    criteria.text = Some(text(&"x".repeat(MAX_PATTERN_BYTES)));
    assert!(check(&criteria).is_ok());

    let mut known = text("x");
    known.encoding = Some("windows-1250".to_owned());
    criteria.text = Some(known);
    assert!(check(&criteria).is_ok());
}

#[test]
fn names_are_bounded_in_length_and_in_masks() {
    let mut criteria = named(&"a".repeat(MAX_PATTERN_BYTES + 1));
    assert_eq!(refused(&criteria), "the name is too long");

    criteria = named(&vec!["m"; MAX_MASKS + 1].join(";"));
    assert_eq!(refused(&criteria), "the name has too many masks");

    criteria = named(&vec!["m"; MAX_MASKS].join(";"));
    assert!(check(&criteria).is_ok());

    // Only masks are counted; a regular expression may hold any number of `;`.
    criteria = named(&vec!["m"; MAX_MASKS + 1].join(";"));
    criteria.name_regex = true;
    assert!(check(&criteria).is_ok());
}

#[test]
fn regular_expressions_that_are_invalid_or_too_big_are_refused_before_a_search_starts() {
    let name = |pattern: &str| {
        let mut criteria = named(pattern);
        criteria.name_regex = true;

        compiled(criteria).err().expect("refused").to_string()
    };
    let content = |pattern: &str| {
        let mut found = text(pattern);
        found.regex = true;

        compiled(FindCriteria {
            text: Some(found),
            ..FindCriteria::default()
        })
        .err()
        .expect("refused")
        .to_string()
    };

    assert_eq!(name("(unclosed"), "the name pattern is not a valid regular expression");
    assert_eq!(name(r"(a+)+\1"), "the name pattern is not a valid regular expression");
    assert_eq!(name(r"\p{L}{1000}"), "the name pattern is too complex");
    assert_eq!(
        name(&format!("{}a{}", "(".repeat(200), ")".repeat(200))),
        "the name pattern is not a valid regular expression"
    );

    assert_eq!(content("(unclosed"), "the text is not valid or is too complex");
    assert_eq!(content(r"\w{100}{100}"), "the text is not valid or is too complex");
    assert_eq!(content(r"a\nb"), "the text is not valid or is too complex");
}

#[test]
fn the_folder_is_checked_by_its_syntax_when_the_search_is_compiled() {
    let query = |root: &str| FindQuery {
        root: root.to_owned(),
        criteria: FindCriteria::default(),
    };

    assert!(compile(&query("relative/path")).is_err());
    assert!(compile(&query(if cfg!(windows) { "C:\\data\\..\\x" } else { "/data/../x" })).is_err());
}
