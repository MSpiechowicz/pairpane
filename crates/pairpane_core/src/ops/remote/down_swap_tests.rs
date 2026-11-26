//! A move removes from the folder the walk went to, however the server answers where a folder is: a folder swapped for a
//! link while the job runs is found by looking at each folder on the way (a server that only cleans a path would not
//! say), and a removal that went elsewhere is reported.

use std::fs;
use std::sync::Arc;

use pretty_assertions::assert_eq;

use crate::model::{OpKind, Outcome};

use super::super::testing::{Rig, long_ago, read, set_times, temp, tree};
use super::testing::{FOLDER_CHANGED, Fake, REMOVED_ELSEWHERE, download, fetch, said};

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
#[cfg(unix)]
fn swap_for_victim(server: &Fake, name: &'static str) -> Arc<dyn Fn() + Send + Sync> {
    let root = server.at("");

    Arc::new(move || {
        fs::rename(root.join(name), root.join(format!("{name}.real"))).expect("moved");
        std::os::unix::fs::symlink(root.join("victim"), root.join(name)).expect("link");
    })
}

#[cfg(unix)]
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

    assert_eq!(said(&ended), [FOLDER_CHANGED]);
    assert_eq!(read(&server.at("victim/s.txt")), b"V", "what the link leads to stays");
    assert_eq!(read(&server.at("sub.real/s.txt")), b"s", "so does the original");
    assert_eq!(tree(local.path()), ["sub/", "sub/s.txt"], "the copy is whole");
}

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

    assert_eq!(said(&ended), [FOLDER_CHANGED]);
    assert_eq!(read(&server.at("victim/s.txt")), b"V");
}

#[cfg(unix)]
#[test]
fn a_removal_that_went_to_another_folder_is_reported() {
    let server = with_victim();

    // The folder is as it was when it is checked, and is a link by the time the server removes the file.
    server.switch(|switches| switches.after_realpath = Some(("/sub".to_owned(), swap_for_victim(&server, "sub"))));
    let local = temp();

    let ended = fetch(&mut Rig::new(), OpKind::Move, &server, &["sub"], local.path());

    assert_eq!(said(&ended), [REMOVED_ELSEWHERE]);
}
