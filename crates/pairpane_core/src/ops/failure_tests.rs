//! A disk that fails, a copy that does not match its source, and a long list of failures.

use pretty_assertions::assert_eq;

use crate::model::{JobPhase, Outcome};

use super::testing::{Rig, has_no_partial_files, pattern, phases, put, read, temp, tree};

#[test]
fn a_failing_disk_fails_the_item_and_removes_its_temporary_file() {
    let root = temp();
    put(root.path(), "src/a.bin", &pattern(2 * 1024 * 1024));
    put(root.path(), "src/b.txt", b"b");
    let mut rig = Rig::hooked(|hooks| hooks.fail_after_data = true);

    let ended = rig.run_copy(root.path(), &["a.bin", "b.txt"]);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 2);
    assert_eq!(tree(&root.path().join("dst")), Vec::<String>::new());
    assert!(has_no_partial_files(root.path()));
}

#[test]
fn verify_passes_for_a_good_copy_and_shows_its_phase() {
    let root = temp();
    put(root.path(), "src/a.bin", &pattern(3 * 1024 * 1024));
    let mut rig = Rig::hooked(|hooks| hooks.every_chunk = true);

    let ended = rig.run_verified(root.path(), &["a.bin"]);

    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(
        read(&root.path().join("dst/a.bin")),
        read(&root.path().join("src/a.bin"))
    );
    assert!(phases(&rig).contains(&JobPhase::Verifying));
}

#[test]
fn a_mismatch_found_by_verify_fails_the_item_and_removes_the_copy() {
    let root = temp();
    put(root.path(), "src/a.bin", &pattern(1024 * 1024));
    let mut rig = Rig::hooked(|hooks| hooks.corrupt_verify = true);

    let ended = rig.run_verified(root.path(), &["a.bin"]);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 1);
    assert_eq!(ended.errors[0].path, "a.bin");
    assert_eq!(ended.errors[0].error.message, "the copy does not match the original");
    assert_eq!(tree(&root.path().join("dst")), Vec::<String>::new());
    assert_eq!(read(&root.path().join("src/a.bin")).len(), 1024 * 1024);
}

#[test]
fn only_the_first_hundred_item_errors_are_listed_and_the_rest_are_counted() {
    let root = temp();
    let names: Vec<String> = (0..105).map(|index| format!("f{index:03}")).collect();
    for name in &names {
        put(root.path(), &format!("src/{name}"), b"x");
    }
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut rig = Rig::hooked(|hooks| hooks.fail_after_data = true);

    let ended = rig.run_copy(root.path(), &names);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 100);
    assert_eq!(ended.more_errors, 5);
    assert!(has_no_partial_files(root.path()));
}
