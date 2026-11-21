use std::fs;

use pretty_assertions::assert_eq;

use super::{Env, purge_below};

#[test]
fn only_the_entries_from_the_folder_are_purged_even_when_its_path_is_written_with_percent_codes() {
    let env = Env::new();
    // The space and the `%` are written as `%20` and `%25` in a record.
    let ours = env.temp.path().join("our folder 100%");
    let other = env.temp.path().join("other folder");
    fs::create_dir(&ours).expect("folder");
    fs::create_dir(&other).expect("folder");
    fs::write(ours.join("a"), "x").expect("file");
    fs::create_dir(ours.join("d")).expect("folder");
    fs::write(ours.join("d/inner"), "x").expect("file");
    fs::write(other.join("b"), "x").expect("file");
    let names = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect::<Vec<_>>();
    super::trash_in(&env.context, &ours, &names(&["a", "d"]))
        .1
        .expect("trashed");
    super::trash_in(&env.context, &other, &names(&["b"]))
        .1
        .expect("trashed");

    purge_below(&env.trash(), &ours);

    assert_eq!(env.listed("files"), ["b"]);
    assert_eq!(env.listed("info"), ["b.trashinfo"]);
}
