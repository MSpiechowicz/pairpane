use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::symlink;

use pretty_assertions::assert_eq;

use super::super::handle::NO_TRASH;
use super::super::restore::untrash;
use super::super::testing::{Env, is_root, me, set_mode};
use super::super::{Context, UntrashFailure};

fn is_no_trash(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::Other && error.to_string() == NO_TRASH
}

#[test]
fn a_file_goes_to_files_with_a_record_and_comes_back() {
    let env = Env::new();
    env.file("a.txt", "old");

    let (record, result) = env.trashed(&["a.txt"]);

    result.expect("trashed");
    assert_eq!(record.names, ["a.txt"]);
    assert!(!env.work().join("a.txt").exists());
    assert_eq!(env.listed("files"), ["a.txt"]);
    assert_eq!(env.listed("info"), ["a.txt.trashinfo"]);
    assert_eq!(
        fs::read_to_string(env.trash().join("files/a.txt")).expect("moved"),
        "old"
    );

    assert_eq!(untrash(&env.context, &record), Ok(1));

    assert_eq!(env.read("a.txt"), "old");
    assert!(env.listed("files").is_empty());
    assert!(env.listed("info").is_empty());
    assert_eq!(untrash(&env.context, &record), Err(UntrashFailure::NothingFound));
}

#[test]
fn the_record_holds_the_absolute_path_and_a_local_date_and_is_private() {
    use std::os::unix::fs::MetadataExt;

    let env = Env::new();
    env.file("with space.txt", "x");

    env.trashed(&["with space.txt"]).1.expect("trashed");

    let path = env.trash().join("info/with space.txt.trashinfo");
    let text = fs::read_to_string(&path).expect("record");
    let canonical = fs::canonicalize(env.work()).expect("canonical");
    let lines: Vec<&str> = text.lines().collect();

    assert_eq!(lines[0], "[Trash Info]");
    assert_eq!(
        lines[1],
        format!("Path={}/with%20space.txt", canonical.to_str().expect("text"))
    );
    let date = lines[2].strip_prefix("DeletionDate=").expect("a date");
    assert_eq!(date.len(), "2026-04-01T12:30:00".len(), "{date}");
    assert_eq!(&date[10..11], "T");
    assert_eq!(lines.len(), 3);
    assert_eq!(fs::metadata(&path).expect("metadata").mode() & 0o777, 0o600);
}

#[test]
fn a_folder_goes_with_what_is_in_it_and_comes_back() {
    let env = Env::new();
    fs::create_dir(env.work().join("d")).expect("folder");
    env.file("d/inner.txt", "inner");

    let (record, result) = env.trashed(&["d"]);

    result.expect("trashed");
    assert!(!env.work().join("d").exists());
    assert_eq!(untrash(&env.context, &record), Ok(1));
    assert_eq!(env.read("d/inner.txt"), "inner");
}

#[test]
fn a_link_to_a_folder_goes_as_the_link_and_the_target_keeps_its_contents() {
    let env = Env::new();
    let target = env.work().join("target");
    fs::create_dir(&target).expect("folder");
    env.file("target/inner.txt", "inner");
    symlink(&target, env.work().join("link")).expect("link");

    let (record, result) = env.trashed(&["link"]);

    result.expect("trashed");
    assert!(fs::symlink_metadata(env.work().join("link")).is_err());
    assert_eq!(env.read("target/inner.txt"), "inner");
    assert!(
        fs::symlink_metadata(env.trash().join("files/link"))
            .expect("the link")
            .is_symlink()
    );

    assert_eq!(untrash(&env.context, &record), Ok(1));

    assert_eq!(fs::read_link(env.work().join("link")).expect("a link again"), target);
    assert_eq!(env.read("target/inner.txt"), "inner");
}

#[test]
fn a_name_that_is_taken_in_the_trash_gets_a_number() {
    let env = Env::new();
    env.file("a", "first");
    let (first, result) = env.trashed(&["a"]);
    result.expect("first");
    env.file("a", "second");
    let (second, result) = env.trashed(&["a"]);
    result.expect("second");

    // One name taken only by a record, one only by an item: both are skipped.
    fs::write(env.trash().join("info/a.3.trashinfo"), "x").expect("record");
    fs::write(env.trash().join("files/a.4"), "x").expect("item");
    env.file("a", "third");
    let (third, result) = env.trashed(&["a"]);
    result.expect("third");

    assert_eq!(
        env.listed("files"),
        ["a", "a.2", "a.4", "a.5"],
        "the items of the three moves, and the one that was there"
    );
    assert_eq!(
        fs::read_to_string(env.trash().join("files/a.5")).expect("third"),
        "third"
    );

    // Each record puts back its own item.
    assert_eq!(untrash(&env.context, &second), Ok(1));
    assert_eq!(env.read("a"), "second");
    fs::remove_file(env.work().join("a")).expect("remove");
    assert_eq!(untrash(&env.context, &first), Ok(1));
    assert_eq!(env.read("a"), "first");
    fs::remove_file(env.work().join("a")).expect("remove");
    assert_eq!(untrash(&env.context, &third), Ok(1));
    assert_eq!(env.read("a"), "third");
}

#[test]
fn a_trash_with_every_name_taken_refuses_and_leaves_no_record() {
    let env = Env::new();
    env.file("a", "x");
    env.trashed(&["a"]).1.expect("first");
    env.file("a", "again");
    let files = env.trash().join("files");
    for number in 2..=1000 {
        fs::write(files.join(format!("a.{number}")), "").expect("item");
    }
    let records = env.listed("info");

    let (record, result) = env.trashed(&["a"]);

    let failure = result.expect_err("full");
    // Our own text, not a clash of names in the user's folder: the page shows it as it is.
    assert_eq!(failure.error.kind(), io::ErrorKind::Other);
    assert_eq!(failure.error.raw_os_error(), None);
    assert_eq!(
        failure.error.to_string(),
        "the Trash holds too many items with this name"
    );
    assert!(record.names.is_empty());
    assert_eq!(env.read("a"), "again");
    assert_eq!(env.listed("info"), records);
}

#[test]
fn a_failed_rename_leaves_no_record_and_the_item_in_place() {
    if is_root() {
        return;
    }
    let env = Env::new();
    env.file("a", "x");
    env.trashed(&["a"]).1.expect("makes the Trash");
    env.file("b", "y");
    let records = env.listed("info");
    set_mode(&env.work(), 0o555);

    let (record, result) = env.trashed(&["b"]);

    set_mode(&env.work(), 0o755);
    let failure = result.expect_err("refused");
    assert_eq!(failure.error.kind(), io::ErrorKind::PermissionDenied);
    assert!(record.names.is_empty());
    assert_eq!(env.read("b"), "y");
    assert_eq!(env.listed("info"), records);
}

#[test]
fn an_item_that_is_gone_is_not_found_and_gets_no_record() {
    let env = Env::new();
    env.file("kept", "x");
    env.trashed(&["kept"]).1.expect("makes the Trash");
    let records = env.listed("info");

    let (record, result) = env.trashed(&["missing"]);

    assert_eq!(result.expect_err("gone").error.kind(), io::ErrorKind::NotFound);
    assert!(record.names.is_empty());
    assert_eq!(env.listed("info"), records);
}

#[test]
fn an_item_that_is_gone_makes_no_trash_folder_at_all() {
    let env = Env::new();

    let (record, result) = env.trashed(&["missing"]);

    assert_eq!(result.expect_err("gone").error.kind(), io::ErrorKind::NotFound);
    assert!(record.names.is_empty());
    assert!(
        !env.temp.path().join("share").exists(),
        "not even the data folder is made"
    );
}

#[test]
fn a_move_stops_at_the_first_failure_and_lists_only_what_went() {
    let env = Env::new();
    env.file("first", "1");
    env.file("last", "3");

    let (record, result) = env.trashed(&["first", "missing", "last"]);

    assert_eq!(result.expect_err("stopped").name, "missing");
    assert_eq!(record.names, ["first"]);
    assert!(env.work().join("last").exists());
    assert_eq!(untrash(&env.context, &record), Ok(1));
}

#[test]
fn a_folder_whose_path_is_not_text_works() {
    let env = Env::new();
    let odd = env.temp.path().join(OsString::from_vec(b"odd-\xff-folder".to_vec()));
    fs::create_dir(&odd).expect("folder");
    fs::write(odd.join("a"), "x").expect("file");
    let names = vec!["a".to_owned()];

    let (record, result) = super::super::trash_in(&env.context, &odd, &names);

    result.expect("trashed");
    let text = fs::read_to_string(env.trash().join("info/a.trashinfo")).expect("record");
    assert!(text.contains("odd-%FF-folder"), "{text}");
    assert_eq!(untrash(&env.context, &record), Ok(1));
    assert!(odd.join("a").exists());
}

#[test]
fn a_trash_folder_that_is_a_link_is_refused_and_nothing_moves() {
    for linked in ["Trash", "Trash/files", "Trash/info"] {
        let env = Env::new();
        env.file("a", "x");
        env.trashed(&["a"]).1.expect("makes the Trash");
        env.file("b", "y");
        let elsewhere = env.temp.path().join("elsewhere");
        fs::create_dir(&elsewhere).expect("folder");
        for name in ["files", "info"] {
            fs::create_dir(elsewhere.join(name)).expect("folder");
        }
        let place = env.temp.path().join("share").join(linked);
        let target = if linked == "Trash" {
            elsewhere.clone()
        } else {
            elsewhere.join(&linked[6..])
        };
        // The real one is moved aside; the link takes its name.
        fs::rename(&place, env.temp.path().join("aside")).expect("aside");
        symlink(&target, &place).expect("link");

        let (record, result) = env.trashed(&["b"]);

        let failure = result.expect_err(linked);
        assert!(is_no_trash(&failure.error), "{linked}: {}", failure.error);
        assert!(record.names.is_empty());
        assert!(env.work().join("b").exists());
        for name in ["files", "info"] {
            assert!(
                fs::read_dir(elsewhere.join(name)).expect("folder").next().is_none(),
                "{linked}"
            );
        }
    }
}

#[test]
fn a_trash_folder_that_others_can_write_to_is_refused_unless_it_is_sticky() {
    let env = Env::new();
    env.file("a", "x");
    env.trashed(&["a"]).1.expect("makes the Trash");

    for (folder, mode, usable) in [
        ("Trash", 0o777, false),
        ("Trash", 0o755, true),
        ("Trash", 0o1777, true),
        ("Trash/files", 0o775, false),
        ("Trash/files", 0o755, true),
        ("Trash/info", 0o707, false),
        ("Trash/info", 0o700, true),
    ] {
        let path = env.temp.path().join("share").join(folder);
        set_mode(&path, mode);
        env.file("b", "y");

        let (record, result) = env.trashed(&["b"]);

        match (result, usable) {
            (Ok(()), true) => {
                assert_eq!(untrash(&env.context, &record), Ok(1), "{folder} {mode:o}");
            }
            (Err(failure), false) => {
                assert!(is_no_trash(&failure.error), "{folder} {mode:o}");
                assert!(env.work().join("b").exists());
            }
            (other, _) => panic!("{folder} {mode:o}: {other:?}"),
        }

        set_mode(&path, 0o700);
        let _ = fs::remove_file(env.work().join("b"));
    }
}

#[test]
fn a_trash_folder_of_another_user_is_refused() {
    let env = Env::new();
    env.file("a", "x");
    env.trashed(&["a"]).1.expect("makes the Trash");
    env.file("b", "y");

    // The folders are the tester's, so to a user with another number they belong to someone else.
    let other = Context {
        uid: me() + 1,
        data_home: env.context.data_home.clone(),
    };
    let names = vec!["b".to_owned()];
    let (record, result) = super::super::trash_in(&other, &env.work(), &names);

    assert!(is_no_trash(&result.expect_err("refused").error));
    assert!(record.names.is_empty());
    assert!(env.work().join("b").exists());
}

/// The real system: an item on a drive that is not the one of the home goes to the Trash in the top folder of its drive,
/// and not to the home Trash, and comes back from there. (Skipped where the temporary folder is on the drive of the
/// home, or the user has no Trash on the drive that holds it.)
#[test]
fn an_item_on_another_drive_than_the_home_goes_to_the_trash_of_its_drive_and_back() {
    use super::super::folders::Kind;
    use super::super::handle::{Facts, open_folder};
    use super::super::top::top_of;

    let Some(home) = std::env::home_dir() else { return };
    let temp = tempfile::TempDir::new().expect("temp dir");
    let work = fs::canonicalize(temp.path()).expect("canonical");
    let fd = |path: &std::path::Path| open_folder(path).expect("folder");
    let (work_top, home_top) = (top_of(&fd(&work)).expect("top"), top_of(&fd(&home)).expect("top"));
    let same = |a: &Facts, b: &Facts| a.is_same_place(b);
    if same(
        &Facts::of_handle(&work_top.fd).expect("facts"),
        &Facts::of_handle(&home_top.fd).expect("facts"),
    ) {
        return;
    }
    let top = work.ancestors().nth(work_top.steps).expect("top path").to_path_buf();
    let own = top.join(format!(".Trash-{}", me()));
    let existed = fs::symlink_metadata(&own).is_ok();
    fs::write(work.join("a"), "x").expect("file");
    let names = vec!["a".to_owned()];

    let (record, result) = super::super::trash(&work, &names);

    let Ok(()) = result else {
        // No Trash that can be used on that drive: not copied to the home Trash instead.
        assert!(work.join("a").exists());
        return;
    };
    let placed = &record.placed[0];
    assert!(matches!(placed.kind, Kind::Own | Kind::Shared));
    assert_eq!(placed.base, top);
    assert_eq!(
        placed.recorded,
        std::path::Path::new(&work.strip_prefix(&top).expect("below").join("a"))
    );
    assert!(!work.join("a").exists());

    assert_eq!(super::super::untrash(&record), Ok(1));
    assert_eq!(fs::read_to_string(work.join("a")).expect("back"), "x");

    if !existed {
        // What the test made is taken away again, when it is empty.
        for part in ["files", "info", ""] {
            let _ = fs::remove_dir(own.join(part));
        }
    }
}

/// The longest name a file system takes is 255 bytes; the record of the item adds `.trashinfo` to the name it has in
/// the Trash.
#[test]
fn a_name_of_the_longest_length_goes_to_the_trash_and_comes_back_whole() {
    let env = Env::new();
    let long = "a".repeat(255);
    env.file(&long, "long");

    let (record, result) = env.trashed(&[&long]);

    result.expect("trashed");
    assert!(!env.work().join(&long).exists());
    for folder in ["files", "info"] {
        let names = env.listed(folder);
        assert_eq!(names.len(), 1, "{folder}");
        assert!(names[0].len() <= 255, "{folder}: {}", names[0].len());
    }
    let text = fs::read_to_string(env.trash().join("info").join(&env.listed("info")[0])).expect("record");
    assert!(text.contains(&format!(
        "Path={}/{long}\n",
        fs::canonicalize(env.work()).expect("path").display()
    )));

    assert_eq!(untrash(&env.context, &record), Ok(1));

    assert_eq!(env.read(&long), "long");
    assert!(env.listed("files").is_empty());
    assert!(env.listed("info").is_empty());
}

#[test]
fn names_that_are_cut_to_the_same_stem_are_told_apart_and_each_comes_back() {
    let env = Env::new();
    let first = "a".repeat(255);
    let second = format!("{}b", "a".repeat(254));
    let third = format!("{}c", "a".repeat(254));
    env.file(&first, "first");
    env.file(&second, "second");
    env.file(&third, "third");

    let (record, result) = env.trashed(&[&first, &second, &third]);

    result.expect("trashed");
    assert_eq!(env.listed("files").len(), 3);
    assert!(env.listed("info").iter().all(|name| name.len() <= 255));

    assert_eq!(untrash(&env.context, &record), Ok(3));

    assert_eq!(env.read(&first), "first");
    assert_eq!(env.read(&second), "second");
    assert_eq!(env.read(&third), "third");
}

#[test]
fn a_long_name_of_text_is_cut_between_characters() {
    let env = Env::new();
    // 127 letters of two bytes each: 254 bytes, and the cut at 245 bytes falls in the middle of a letter.
    let text = "\u{e9}".repeat(127);
    env.file(&text, "text");

    let (record, result) = env.trashed(&[&text]);

    result.expect("trashed");
    let stored = env.listed("files");
    assert_eq!(stored.len(), 1);
    assert!(!stored[0].contains('\u{FFFD}'), "a letter was cut in two");
    assert_eq!(stored[0].len(), 244);
    assert_eq!(untrash(&env.context, &record), Ok(1));
    assert_eq!(env.read(&text), "text");
}

#[test]
fn a_long_name_that_is_not_text_is_cut_at_a_byte() {
    let mut bytes = vec![b'x'; 250];
    bytes.extend_from_slice(b"\xff\xfe\xfd\xfc\xfb");
    let odd = OsString::from_vec(bytes);

    assert_eq!(super::stem(&odd, 1).len(), 245);
    assert_eq!(super::stem(&odd, 2).len(), 245, "the number is part of the room");
}

#[test]
fn a_stem_leaves_room_for_the_record_suffix_and_the_number() {
    use std::ffi::OsStr;

    let long = OsStr::new(&"n".repeat(255)).to_owned();

    for (number, suffix) in [(1, ""), (2, ".2"), (10, ".10"), (1000, ".1000")] {
        let stem = super::stem(&long, number);

        assert!(stem.len() + ".trashinfo".len() <= 255, "{number}");
        assert!(stem.to_string_lossy().ends_with(suffix), "{number}");
    }
    assert_eq!(super::stem(OsStr::new("short"), 1), "short");
    assert_eq!(super::stem(OsStr::new("short"), 3), "short.3");
}
