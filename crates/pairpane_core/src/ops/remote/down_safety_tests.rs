//! A download must not be turned against the server or this computer by what the server does while it runs: a folder
//! swapped for a link, a listing too large to hold, two names that meet at one name here.

use std::fs;
use std::sync::Arc;

use pretty_assertions::assert_eq;

use crate::model::{Answer, OpKind, Outcome};

use super::super::testing::{Rig, long_ago, read, set_times, temp, tree};
use super::testing::{FOLDER_CHANGED as CHANGED, Fake, download, failure, fetch, said};

/// The server swaps its folder `sub` for a link to its folder `victim` when the first file is read.
#[cfg(unix)]
fn swap_sub_for_victim_at_first_read(server: &Fake) {
    let root = server.at("");

    server.switch(|switches| {
        switches.at_first_read = Some(Arc::new(move || {
            fs::remove_dir_all(root.join("sub")).expect("removed");
            std::os::unix::fs::symlink(root.join("victim"), root.join("sub")).expect("link");
        }));
    });
}

#[cfg(unix)]
#[test]
fn a_folder_swapped_for_a_link_before_it_is_walked_is_not_followed_and_nothing_behind_it_is_removed() {
    let server = Fake::new();
    server.put("a.txt", b"a");
    server.put("sub/s.txt", b"s");
    server.put("victim/v.txt", b"victim data");
    swap_sub_for_victim_at_first_read(&server);
    let local = temp();

    let ended = fetch(&mut Rig::new(), OpKind::Move, &server, &["a.txt", "sub"], local.path());

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 1, "{:?}", said(&ended));
    assert_eq!(ended.errors[0].path, "sub");
    assert_eq!(
        read(&server.at("victim/v.txt")),
        b"victim data",
        "what the link leads to stays"
    );
    assert_eq!(tree(local.path()), ["a.txt"], "nothing of the other folder came here");
}

#[cfg(unix)]
#[test]
fn a_folder_swapped_for_a_link_after_it_was_listed_keeps_the_original_of_a_file_that_matches() {
    let server = Fake::new();
    server.put("sub/s.txt", b"s");
    // The same size and time, so that nothing but where the folder is tells the file apart.
    server.put("victim/s.txt", b"s");
    for path in ["sub/s.txt", "victim/s.txt"] {
        set_times(&server.at(path), long_ago());
    }
    swap_sub_for_victim_at_first_read(&server);
    let local = temp();

    let ended = fetch(&mut Rig::new(), OpKind::Move, &server, &["sub"], local.path());

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(said(&ended), [CHANGED]);
    assert_eq!(
        read(&server.at("victim/s.txt")),
        b"s",
        "the file behind the link is not removed"
    );
    assert_eq!(read(&local.path().join("sub/s.txt")), b"s", "the copy is whole");
}

#[test]
fn a_move_from_a_server_that_cannot_say_where_a_folder_is_is_refused_and_a_copy_is_not() {
    let server = Fake::new();
    server.put("a.txt", b"a");
    server.switch(|switches| switches.no_realpath = true);

    let local = temp();
    let ended = fetch(&mut Rig::new(), OpKind::Move, &server, &["a.txt"], local.path());

    assert_eq!(
        failure(&ended),
        "this server cannot tell where a folder is, so its items cannot be moved"
    );
    assert_eq!(tree(local.path()), Vec::<String>::new());
    assert_eq!(tree(&server.at("")), ["a.txt"]);

    let ended = fetch(&mut Rig::new(), OpKind::Copy, &server, &["a.txt"], local.path());

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(tree(local.path()), ["a.txt"]);
}

/// A server with `levels` nested folders, each with `files` files.
fn nested(levels: usize, files: usize) -> Arc<Fake> {
    let server = Fake::new();
    let mut folder = String::from("top");

    for _ in 0..levels {
        for number in 0..files {
            server.put(&format!("{folder}/{number:03}.txt"), b"x");
        }

        folder.push_str("/in");
    }

    server
}

fn limited(held: usize, bytes: usize, walked: usize) -> Rig {
    Rig::hooked(|hooks| hooks.walk_limits = Some((held, bytes, walked)))
}

#[test]
fn folders_nested_so_that_their_listings_pass_the_limit_fail_the_job_and_write_nothing() {
    let server = nested(3, 10);
    let before = tree(&server.at(""));
    let local = temp();

    let ended = fetch(
        &mut limited(25, usize::MAX, usize::MAX),
        OpKind::Move,
        &server,
        &["top"],
        local.path(),
    );

    assert_eq!(failure(&ended), "the folder on the server is too large to copy at once");
    assert_eq!(tree(local.path()), Vec::<String>::new());
    assert_eq!(tree(&server.at("")), before, "nothing was removed");
}

#[test]
fn listings_that_are_given_back_do_not_add_up() {
    let server = Fake::new();
    for folder in ["a", "b", "c", "d"] {
        for number in 0..10 {
            server.put(&format!("{folder}/{number}.txt"), b"x");
        }
    }
    let local = temp();

    let ended = fetch(
        &mut limited(15, usize::MAX, usize::MAX),
        OpKind::Copy,
        &server,
        &["a", "b", "c", "d"],
        local.path(),
    );

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 0, "{:?}", said(&ended));
    assert_eq!(tree(local.path()).len(), 4 + 40);
}

#[test]
fn the_names_a_walk_holds_and_the_entries_it_lists_in_all_are_bounded_too() {
    let server = Fake::new();
    for number in 0..30 {
        server.put(&format!("d/{number:030}.txt"), b"x");
    }
    let local = temp();

    for (limits, fails) in [
        ((usize::MAX, 29 * 34, usize::MAX), true),
        ((usize::MAX, 30 * 34, usize::MAX), false),
        ((usize::MAX, usize::MAX, 29), true),
        ((usize::MAX, usize::MAX, 30), false),
    ] {
        let (held, bytes, walked) = limits;
        let ended = fetch(
            &mut limited(held, bytes, walked),
            OpKind::Copy,
            &server,
            &["d"],
            local.path(),
        );

        if fails {
            assert_eq!(
                failure(&ended),
                "the folder on the server is too large to copy at once",
                "{limits:?}"
            );
        } else {
            assert_eq!(ended.outcome, Outcome::Done, "{limits:?}");
            fs::remove_dir_all(local.path().join("d")).expect("removed");
        }
    }
}

/// Two names of the server that a disk with no difference of case gives one name: `A.txt` and `a.txt` here, as the
/// download names what it writes (the server of the test is case-sensitive, so only Linux can hold both).
#[cfg(target_os = "linux")]
mod meeting {
    use std::fs;
    use std::sync::Arc;

    use pretty_assertions::assert_eq;

    use crate::model::{Answer, OpKind, Outcome};

    use super::super::super::testing::{Rig, long_ago, read, set_times, temp, tree};
    use super::super::testing::{Fake, download, fetch, said};

    fn rig() -> Rig {
        Rig::hooked(|hooks| hooks.fold_names = true)
    }

    fn two_files() -> Arc<Fake> {
        let server = Fake::new();
        server.put("A.txt", b"first");
        server.put("a.txt", b"second");

        server
    }

    #[test]
    fn the_second_of_two_items_with_one_name_is_not_asked_about_and_never_replaces_the_first() {
        let server = two_files();
        set_times(&server.at("A.txt"), long_ago());
        let local = temp();

        let ended = fetch(&mut rig(), OpKind::Move, &server, &["A.txt", "a.txt"], local.path());

        assert_eq!(ended.outcome, Outcome::Done);
        assert_eq!(
            said(&ended),
            ["two items on the server have the same name on this computer"]
        );
        assert_eq!(read(&local.path().join("a.txt")), b"first");
        assert_eq!(tree(&server.at("")), ["a.txt"], "the original of the second stays");
    }

    #[test]
    fn an_answer_to_replace_all_that_was_given_before_does_not_replace_what_the_job_wrote() {
        let server = two_files();
        server.put("x.txt", b"from the server");
        let local = temp();
        fs::write(local.path().join("x.txt"), b"mine").expect("file");
        let mut rig = rig();

        let job = rig.start(download(
            OpKind::Move,
            &server,
            "/",
            &["x.txt", "A.txt", "a.txt"],
            local.path(),
        ));
        rig.answer_next(job, Answer::Overwrite, true);
        let ended = rig.wait_ended(job);

        assert_eq!(
            said(&ended),
            ["two items on the server have the same name on this computer"]
        );
        assert_eq!(read(&local.path().join("x.txt")), b"from the server");
        assert_eq!(read(&local.path().join("a.txt")), b"first");
        assert_eq!(tree(&server.at("")), ["a.txt"]);
    }

    #[test]
    fn two_folders_with_one_name_are_not_merged_and_the_second_stays_on_the_server() {
        let server = Fake::new();
        server.put("Dir/x.txt", b"x");
        server.put("dir/y.txt", b"y");
        let local = temp();

        let ended = fetch(&mut rig(), OpKind::Move, &server, &["Dir", "dir"], local.path());

        assert_eq!(ended.outcome, Outcome::Done);
        assert_eq!(
            said(&ended),
            ["two items on the server have the same name on this computer"]
        );
        assert_eq!(tree(local.path()), ["dir/", "dir/x.txt"]);
        assert_eq!(tree(&server.at("")), ["dir/", "dir/y.txt"]);
    }
}

#[test]
fn a_target_whose_identity_cannot_be_read_is_not_replaced_by_a_move_but_is_by_a_copy() {
    for (kind, replaced) in [(OpKind::Move, false), (OpKind::Copy, true)] {
        let server = Fake::new();
        server.put("a.txt", b"from the server");
        let local = temp();
        fs::write(local.path().join("a.txt"), b"mine").expect("file");
        let mut rig = Rig::hooked(|hooks| hooks.unreadable_identity = true);

        let job = rig.start(download(kind, &server, "/", &["a.txt"], local.path()));
        rig.answer_next(job, Answer::Overwrite, false);
        let ended = rig.wait_ended(job);

        assert_eq!(ended.errors.len(), usize::from(!replaced), "{kind:?}");
        assert_eq!(
            read(&local.path().join("a.txt")),
            if replaced { &b"from the server"[..] } else { b"mine" },
            "{kind:?}"
        );
        assert!(
            server.at("a.txt").exists(),
            "{kind:?}: a move that replaced nothing removes nothing"
        );
    }
}

#[test]
fn the_file_laid_to_tell_an_alias_has_a_name_that_says_nothing_of_this_computer() {
    let server = Fake::new();
    server.put("a.txt", b"a");
    let local = temp();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let watched = local.path().to_owned();
    let into = Arc::clone(&seen);

    server.switch(|switches| {
        switches.at_first_read = Some(Arc::new(move || {
            let names = fs::read_dir(&watched)
                .expect("folder")
                .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned());

            into.lock()
                .expect("names")
                .extend(names.filter(|name| name.ends_with("-probe")));
        }));
    });

    let ended = Rig::new().run(download(OpKind::Copy, &server, "/", &["a.txt"], local.path()));
    let names = seen.lock().expect("names").clone();

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(names.len(), 1, "{names:?}");

    let token = names[0]
        .strip_prefix(".pairpane-")
        .and_then(|rest| rest.strip_suffix("-probe"));

    assert!(
        token.is_some_and(|token| token.len() == 16 && token.chars().all(|char| char.is_ascii_hexdigit())),
        "only a random number: {names:?}"
    );
    assert_eq!(tree(local.path()), ["a.txt"], "the probe is removed");
}
