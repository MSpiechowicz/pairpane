//! (Unix only: they swap folders for symbolic links.)
//!
//! A move removes from the folder the walk went to, however the server answers where a folder is: a folder swapped for a
//! link while the job runs is found by looking at each folder on the way (a server that only cleans a path would not
//! say), and a removal that went elsewhere is reported.

use std::fs;
use std::sync::Arc;

use pretty_assertions::assert_eq;

use crate::model::{OpKind, Outcome};

use super::super::testing::{Rig, long_ago, read, set_times, temp, tree};
use super::testing::{FOLDER_CHANGED, Fake, NOT_LISTED, REMOVED_ELSEWHERE, download, failure, fetch, said};

/// A server with the file `s.txt` in `sub`, and a file of the same name, size and time in `victim`.
fn with_victim() -> Arc<Fake> {
    let server = Fake::new();

    server.put("sub/s.txt", b"s");
    server.put("victim/s.txt", b"V");
    for path in ["sub/s.txt", "victim/s.txt"] {
        set_times(&server.at(path), long_ago());
    }

    server
}

/// The folder `name` of the server is moved aside, and `name` is made a link to its folder `victim`.
fn swap_for_victim(server: &Fake, name: &'static str) -> Arc<dyn Fn() + Send + Sync> {
    let root = server.at("");

    Arc::new(move || {
        fs::rename(root.join(name), root.join(format!("{name}.real"))).expect("moved");
        std::os::unix::fs::symlink(root.join("victim"), root.join(name)).expect("link");
    })
}

#[test]
fn a_server_that_follows_no_link_to_say_where_a_folder_is_does_not_hide_a_swap_of_it() {
    let server = with_victim();
    let swap = swap_for_victim(&server, "sub");

    server.switch(|switches| {
        switches.lexical_realpath = true;
        switches.at_first_read = Some(swap);
    });
    let local = temp();

    let ended = fetch(&mut Rig::new(), OpKind::Move, &server, &["sub"], local.path());

    assert_eq!(said(&ended), [FOLDER_CHANGED, NOT_LISTED]);
    assert_eq!(read(&server.at("victim/s.txt")), b"V", "what the link leads to stays");
    assert_eq!(read(&server.at("sub.real/s.txt")), b"s", "so does the original");
    assert_eq!(tree(local.path()), ["sub/", "sub/s.txt"], "the copy is whole");
}

#[test]
fn the_folder_of_the_items_swapped_for_a_link_during_the_job_keeps_the_originals() {
    for lexical in [true, false] {
        let server = with_victim();
        let swap = swap_for_victim(&server, "sub");

        server.switch(|switches| {
            switches.lexical_realpath = lexical;
            switches.at_first_read = Some(swap);
        });
        let local = temp();

        let ended = Rig::new().run(download(OpKind::Move, &server, "/sub", &["s.txt"], local.path()));

        assert_eq!(said(&ended), [FOLDER_CHANGED], "lexical: {lexical}");
        assert_eq!(read(&server.at("victim/s.txt")), b"V", "lexical: {lexical}");
        assert_eq!(read(&server.at("sub.real/s.txt")), b"s", "lexical: {lexical}");
    }
}

#[test]
fn a_folder_that_is_itself_a_link_the_user_chose_can_be_moved_from() {
    for lexical in [true, false] {
        let server = Fake::new();

        server.put("real/a.txt", b"a");
        server.put("real/in/b.txt", b"b");
        std::os::unix::fs::symlink(server.at("real"), server.at("chosen")).expect("link");
        server.switch(|switches| switches.lexical_realpath = lexical);
        let local = temp();

        let ended = Rig::new().run(download(
            OpKind::Move,
            &server,
            "/chosen",
            &["a.txt", "in"],
            local.path(),
        ));

        assert_eq!(ended.outcome, Outcome::Done, "lexical: {lexical}");
        assert_eq!(ended.errors.len(), 0, "{:?}, lexical: {lexical}", said(&ended));
        assert_eq!(tree(local.path()), ["a.txt", "in/", "in/b.txt"], "lexical: {lexical}");
        assert_eq!(tree(&server.at("real")), Vec::<String>::new(), "lexical: {lexical}");
    }
}

#[test]
fn a_folder_below_a_link_the_user_chose_is_not_moved_from_once_it_is_a_link_itself() {
    let server = with_victim();
    let swap = {
        let root = server.at("");

        Arc::new(move || {
            fs::rename(root.join("real/sub"), root.join("real/sub.real")).expect("moved");
            std::os::unix::fs::symlink(root.join("victim"), root.join("real/sub")).expect("link");
        })
    };

    fs::create_dir(server.at("real")).expect("folder");
    fs::rename(server.at("sub"), server.at("real/sub")).expect("moved");
    std::os::unix::fs::symlink(server.at("real"), server.at("chosen")).expect("link");
    server.switch(|switches| {
        switches.lexical_realpath = true;
        switches.at_first_read = Some(swap);
    });
    let local = temp();

    let ended = Rig::new().run(download(OpKind::Move, &server, "/chosen", &["sub"], local.path()));

    assert_eq!(said(&ended), [FOLDER_CHANGED, NOT_LISTED]);
    assert_eq!(read(&server.at("victim/s.txt")), b"V");
}

#[test]
fn a_removal_that_went_to_another_folder_is_reported() {
    let server = with_victim();

    // The folder is as it was when it is checked, and is a link by the time the server removes the file.
    server.switch(|switches| switches.after_realpath = Some(("/sub".to_owned(), swap_for_victim(&server, "sub"))));
    let local = temp();

    let ended = fetch(&mut Rig::new(), OpKind::Move, &server, &["sub"], local.path());

    assert_eq!(said(&ended), [REMOVED_ELSEWHERE, NOT_LISTED]);
}

/// A server with `s.txt` in `real/` and `vroot/`, shaped alike (the file has one byte, `r` or `v`), so that what a swap
/// leads to has a file to lose.
fn with_two_roots(shape: &str) -> Arc<Fake> {
    let server = Fake::new();

    for root in ["real", "vroot"] {
        server.put(&format!("{root}/{shape}s.txt"), &root.as_bytes()[..1]);
        set_times(&server.at(&format!("{root}/{shape}s.txt")), long_ago());
    }

    server
}

#[test]
fn a_link_the_user_chose_for_the_job_folder_that_is_repointed_during_the_job_keeps_the_originals() {
    for lexical in [true, false] {
        let server = with_two_roots("");
        let root = server.at("");

        std::os::unix::fs::symlink(root.join("real"), root.join("chosen")).expect("link");
        server.switch(|switches| {
            switches.lexical_realpath = lexical;
            switches.at_first_read = Some(Arc::new(move || {
                fs::remove_file(root.join("chosen")).expect("link removed");
                std::os::unix::fs::symlink(root.join("vroot"), root.join("chosen")).expect("repointed");
            }));
        });
        let local = temp();

        let ended = Rig::new().run(download(OpKind::Move, &server, "/chosen", &["s.txt"], local.path()));

        assert_eq!(said(&ended), [FOLDER_CHANGED], "lexical: {lexical}");
        assert_eq!(read(&server.at("vroot/s.txt")), b"v", "lexical: {lexical}");
        assert_eq!(read(&server.at("real/s.txt")), b"r", "lexical: {lexical}");
    }
}

#[test]
fn a_folder_above_the_job_folder_swapped_for_a_link_during_the_job_keeps_the_originals() {
    for lexical in [true, false] {
        let server = with_two_roots("inbox/");
        let root = server.at("");

        fs::rename(root.join("real"), root.join("team")).expect("moved");
        server.switch(|switches| {
            switches.lexical_realpath = lexical;
            switches.at_first_read = Some(Arc::new(move || {
                fs::rename(root.join("team"), root.join("team.real")).expect("moved aside");
                std::os::unix::fs::symlink(root.join("vroot"), root.join("team")).expect("link");
            }));
        });
        let local = temp();

        let ended = Rig::new().run(download(OpKind::Move, &server, "/team/inbox", &["s.txt"], local.path()));

        assert_eq!(said(&ended), [FOLDER_CHANGED], "lexical: {lexical}");
        assert_eq!(read(&server.at("vroot/inbox/s.txt")), b"v", "lexical: {lexical}");
        assert_eq!(read(&server.at("team.real/inbox/s.txt")), b"r", "lexical: {lexical}");
    }
}

#[test]
fn a_move_through_a_link_is_refused_by_a_server_that_cannot_say_where_a_link_points_but_a_copy_is_not() {
    for (kind, refused) in [(OpKind::Move, true), (OpKind::Copy, false)] {
        let server = with_two_roots("");

        std::os::unix::fs::symlink(server.at("real"), server.at("chosen")).expect("link");
        server.switch(|switches| switches.no_read_link = true);
        let local = temp();

        let ended = Rig::new().run(download(kind, &server, "/chosen", &["s.txt"], local.path()));

        if refused {
            assert_eq!(
                failure(&ended),
                "this server cannot tell where a link points, so its items cannot be moved through one"
            );
            assert_eq!(tree(local.path()), Vec::<String>::new());
        } else {
            assert_eq!(ended.outcome, Outcome::Done);
            assert_eq!(tree(local.path()), ["s.txt"]);
        }
        assert_eq!(read(&server.at("real/s.txt")), b"r", "{kind:?}");
    }
}
