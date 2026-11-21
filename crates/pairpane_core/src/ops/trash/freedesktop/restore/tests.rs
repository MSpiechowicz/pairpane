use std::fs;
use std::os::unix::fs::symlink;

use pretty_assertions::assert_eq;

use super::super::handle::{Facts, open_folder};
use super::super::testing::{Env, set_mode};
use super::super::{Trashed, UntrashFailure};
use super::{restore_one, untrash};

/// A change to the record of the item in the Trash.
type Tamper = fn(&Env);

/// A place with the file `a.txt` ("old") in the Trash.
fn trashed_a() -> (Env, Trashed) {
    let env = Env::new();
    env.file("a.txt", "old");
    let (record, result) = env.trashed(&["a.txt"]);
    result.expect("trashed");

    (env, record)
}

fn in_the_trash(env: &Env) -> bool {
    fs::symlink_metadata(env.trash().join("files/a.txt")).is_ok()
}

#[test]
fn a_new_item_at_the_old_place_blocks_the_restore_until_it_is_gone() {
    let (env, record) = trashed_a();
    env.file("a.txt", "new");

    let blocked = untrash(&env.context, &record);

    assert_eq!(
        blocked,
        Err(UntrashFailure::InTheWay {
            name: "a.txt".to_owned()
        })
    );
    assert_eq!(env.read("a.txt"), "new");
    assert!(in_the_trash(&env));

    fs::remove_file(env.work().join("a.txt")).expect("remove");

    assert_eq!(untrash(&env.context, &record), Ok(1));
    assert_eq!(env.read("a.txt"), "old");
}

#[test]
fn a_link_that_leads_nowhere_at_the_old_place_blocks_the_restore_and_is_kept() {
    let (env, record) = trashed_a();
    symlink(env.work().join("nowhere"), env.work().join("a.txt")).expect("link");

    let blocked = untrash(&env.context, &record);

    assert!(matches!(blocked, Err(UntrashFailure::InTheWay { .. })));
    assert!(
        fs::symlink_metadata(env.work().join("a.txt"))
            .expect("kept")
            .is_symlink()
    );
    assert!(in_the_trash(&env));
}

#[test]
fn a_replaced_folder_is_not_put_back_into_and_the_item_stays_in_the_trash() {
    let (env, record) = trashed_a();
    let work = env.work();
    fs::rename(&work, env.temp.path().join("moved-away")).expect("rename");
    fs::create_dir(&work).expect("same name again");

    assert_eq!(untrash(&env.context, &record), Err(UntrashFailure::FolderChanged));

    assert!(!work.join("a.txt").exists());
    assert!(in_the_trash(&env));
    assert!(env.trash().join("info/a.txt.trashinfo").exists());

    // The folder itself coming back makes the undo work again.
    fs::remove_dir(&work).expect("remove");
    fs::rename(env.temp.path().join("moved-away"), &work).expect("rename back");
    assert_eq!(untrash(&env.context, &record), Ok(1));
}

#[test]
fn a_folder_that_is_gone_is_not_made_again() {
    let (env, record) = trashed_a();
    fs::remove_dir(env.work()).expect("remove");

    assert_eq!(untrash(&env.context, &record), Err(UntrashFailure::FolderChanged));

    assert!(!env.work().exists());
    assert!(in_the_trash(&env));
}

#[test]
fn an_item_that_was_emptied_from_the_trash_is_nothing_to_restore() {
    let (env, record) = trashed_a();
    fs::remove_file(env.trash().join("files/a.txt")).expect("empty");

    assert_eq!(untrash(&env.context, &record), Err(UntrashFailure::NothingFound));
}

#[test]
fn a_trash_that_is_gone_is_nothing_to_restore() {
    let (env, record) = trashed_a();
    fs::remove_dir_all(env.trash()).expect("empty");

    assert_eq!(untrash(&env.context, &record), Err(UntrashFailure::NothingFound));
}

#[test]
fn a_record_that_does_not_say_what_was_written_stops_the_restore() {
    let tampers: [(&str, Tamper); 5] = [
        ("another path", |env| {
            fs::write(
                env.trash().join("info/a.txt.trashinfo"),
                "[Trash Info]\nPath=/etc/passwd\nDeletionDate=2026-01-01T00:00:00\n",
            )
            .expect("write");
        }),
        ("no header", |env| {
            fs::write(env.trash().join("info/a.txt.trashinfo"), "Path=/x\n").expect("write");
        }),
        ("too long", |env| {
            let filler = "#".repeat(20_000);
            let path = env.trash().join("info/a.txt.trashinfo");
            let mut text = fs::read_to_string(&path).expect("read");
            text.push_str(&filler);
            fs::write(&path, text).expect("write");
        }),
        ("a link", |env| {
            let path = env.trash().join("info/a.txt.trashinfo");
            let copy = env.temp.path().join("copy");
            fs::rename(&path, &copy).expect("aside");
            symlink(&copy, &path).expect("link");
        }),
        ("missing", |env| {
            fs::remove_file(env.trash().join("info/a.txt.trashinfo")).expect("remove");
        }),
    ];

    for (what, tamper) in tampers {
        let (env, record) = trashed_a();
        tamper(&env);

        let result = untrash(&env.context, &record);

        assert_eq!(result, Err(UntrashFailure::Other), "{what}");
        assert!(!env.work().join("a.txt").exists(), "{what}");
        assert!(in_the_trash(&env), "{what}");
    }
}

#[test]
fn a_trash_folder_that_became_unusable_stops_the_restore() {
    let (env, record) = trashed_a();
    set_mode(&env.trash().join("files"), 0o777);

    assert_eq!(untrash(&env.context, &record), Err(UntrashFailure::Other));

    assert!(!env.work().join("a.txt").exists());
    assert!(in_the_trash(&env));
}

#[test]
fn a_restore_puts_back_what_it_can_and_stops_at_the_item_in_the_way() {
    let env = Env::new();
    for name in ["a", "b", "c"] {
        env.file(name, name);
    }
    let (record, result) = env.trashed(&["a", "b", "c"]);
    result.expect("trashed");
    env.file("b", "new b");

    let result = untrash(&env.context, &record);

    assert_eq!(result, Err(UntrashFailure::InTheWay { name: "b".to_owned() }));
    assert_eq!(env.read("a"), "a");
    assert_eq!(env.read("b"), "new b");
    assert!(!env.work().join("c").exists());

    // A retry finds only the items that are left.
    fs::remove_file(env.work().join("b")).expect("remove");
    assert_eq!(untrash(&env.context, &record), Ok(2));
    assert_eq!(env.read("c"), "c");
}

#[test]
fn a_record_with_no_items_has_nothing_to_restore() {
    let env = Env::new();

    assert_eq!(
        untrash(&env.context, &Trashed::default()),
        Err(UntrashFailure::NothingFound)
    );
}

#[test]
fn a_folder_that_vanished_while_it_was_open_leaves_the_item_in_the_trash_and_the_record_kept() {
    let (env, record) = trashed_a();
    let folder = open_folder(&env.work()).expect("folder");
    let here = Facts::of_handle(&folder).expect("facts");
    // The folder goes between the moment it was opened and the move back.
    fs::remove_dir(env.work()).expect("remove");

    let result = restore_one(&env.context, &record.placed[0], &folder, &here);

    assert_eq!(result, Err(UntrashFailure::FolderChanged));
    assert!(in_the_trash(&env));
    assert!(env.trash().join("info/a.txt.trashinfo").exists());
}

#[test]
fn a_named_pipe_in_place_of_a_record_stops_the_restore_and_does_not_wait_for_a_writer() {
    use std::sync::mpsc;
    use std::time::Duration;

    use rustix::fs::{CWD, FileType, Mode, mknodat};

    let (env, record) = trashed_a();
    let path = env.trash().join("info/a.txt.trashinfo");
    fs::remove_file(&path).expect("remove");
    mknodat(CWD, &path, FileType::Fifo, Mode::RUSR | Mode::WUSR, 0).expect("a pipe");

    // A restore that waits would never answer, so it runs on a thread that is waited for a while.
    let (sender, receiver) = mpsc::channel();
    let context = env.context.clone();
    std::thread::spawn(move || {
        let _ = sender.send(untrash(&context, &record));
    });
    let result = receiver.recv_timeout(Duration::from_secs(10));

    assert_eq!(result, Ok(Err(UntrashFailure::Other)));
    assert!(!env.work().join("a.txt").exists());
    assert!(in_the_trash(&env));
}

#[test]
fn a_folder_with_the_same_device_and_inode_but_another_owner_is_not_the_folder_of_the_item() {
    let (env, mut record) = trashed_a();
    record.placed[0].folder_owner += 1;

    assert_eq!(untrash(&env.context, &record), Err(UntrashFailure::FolderChanged));

    assert!(in_the_trash(&env));
    assert!(!env.work().join("a.txt").exists());
}

#[test]
fn a_folder_made_at_another_time_is_not_the_folder_of_the_item_though_its_inode_is_the_same() {
    let (env, mut record) = trashed_a();
    let Some(birth) = record.placed[0].folder_birth else {
        // A file system that does not tell when a folder was made.
        return;
    };

    // The folder is replaced by a new one, and the record is made to say that the new one has the old number.
    fs::remove_dir(env.work()).expect("remove");
    std::thread::sleep(std::time::Duration::from_millis(50));
    fs::create_dir(env.work()).expect("same name again");
    let new = Facts::of_handle(&open_folder(&env.work()).expect("folder")).expect("facts");
    record.placed[0].folder_device = new.device;
    record.placed[0].folder_inode = new.inode;
    assert_ne!(
        record.placed[0].folder_birth,
        super::birth_of(&open_folder(&env.work()).expect("folder"))
    );
    assert_eq!(record.placed[0].folder_birth, Some(birth));

    assert_eq!(untrash(&env.context, &record), Err(UntrashFailure::FolderChanged));

    assert!(in_the_trash(&env));
}

#[test]
fn a_birth_time_that_is_not_known_on_one_side_is_not_held_against_the_folder() {
    let (env, mut record) = trashed_a();
    record.placed[0].folder_birth = None;

    assert_eq!(untrash(&env.context, &record), Ok(1));
}
