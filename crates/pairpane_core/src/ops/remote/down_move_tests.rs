//! A move from a server: the original is removed only once its copy is in place and it is seen unchanged; anything that
//! cannot be told keeps it.

use std::fs;

use pretty_assertions::assert_eq;

use crate::model::{Answer, OpKind, Outcome};

use super::super::testing::{Rig, has_no_partial_files, pattern, read, temp, tree};
use super::testing::{Fake, Grow, answered, claim_meanwhile, download, fetch, said};

const KEPT: &str = "Kept the original: it changed while it was being moved";

#[test]
fn a_moved_tree_leaves_the_server_empty_and_lands_whole() {
    let server = Fake::new();
    server.put("docs/a.txt", b"alpha");
    server.put("docs/deep/b.bin", &pattern(300_000));
    server.put("c.txt", b"c");
    let local = temp();

    let ended = fetch(&mut Rig::new(), OpKind::Move, &server, &["docs", "c.txt"], local.path());

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 0, "{:?}", ended.errors);
    assert_eq!(read(&local.path().join("docs/deep/b.bin")), pattern(300_000));
    assert_eq!(
        tree(&server.at("")),
        Vec::<String>::new(),
        "the files and the folders are gone"
    );
    assert_eq!(server.handles(), 0);
}

#[test]
fn a_file_that_was_not_copied_stays_on_the_server_and_so_does_its_folder() {
    let server = Fake::new();
    server.put("d/taken.txt", b"server");
    server.put("d/free.txt", b"free");
    let local = temp();
    fs::create_dir(local.path().join("d")).expect("folder");
    fs::write(local.path().join("d/taken.txt"), b"mine").expect("file");
    let request = download(OpKind::Move, &server, "/", &["d"], local.path());

    let (_, ended) = answered(&mut Rig::new(), request, Answer::Skip);

    assert_eq!((ended.skipped, ended.errors.len()), (1, 0));
    assert_eq!(tree(&server.at("")), ["d/", "d/taken.txt"]);
    assert_eq!(read(&local.path().join("d/taken.txt")), b"mine");
    assert_eq!(read(&local.path().join("d/free.txt")), b"free");
}

#[cfg(unix)]
#[test]
fn a_folder_with_a_link_in_it_is_not_removed() {
    let server = Fake::new();
    server.put("d/a.txt", b"a");
    std::os::unix::fs::symlink("a.txt", server.at("d/link")).expect("link");
    let local = temp();

    let ended = fetch(&mut Rig::new(), OpKind::Move, &server, &["d"], local.path());

    assert_eq!((ended.skipped, ended.errors.len()), (1, 0));
    assert_eq!(tree(&server.at("")), ["d/", "d/link"]);
}

#[test]
fn a_removal_the_server_refuses_is_reported_and_the_copy_stays() {
    let server = Fake::new();
    server.put("a.txt", b"a");
    server.switch(|switches| switches.fail_remove = true);
    let local = temp();

    let ended = fetch(&mut Rig::new(), OpKind::Move, &server, &["a.txt"], local.path());

    assert_eq!(
        said(&ended),
        ["The copy was made, but the original may still be on the server: the server refused to remove it"]
    );
    assert_eq!(tree(&server.at("")), ["a.txt"]);
    assert_eq!(read(&local.path().join("a.txt")), b"a");
}

#[test]
fn an_original_that_was_gone_by_the_time_of_its_removal_is_said_to_be_gone_not_to_be_still_there() {
    let server = Fake::new();
    server.put("a.txt", b"a");
    fs::create_dir(server.at("empty")).expect("folder");
    server.switch(|switches| switches.remove_finds_nothing = true);
    let local = temp();

    let ended = fetch(
        &mut Rig::new(),
        OpKind::Move,
        &server,
        &["a.txt", "empty"],
        local.path(),
    );

    assert_eq!(
        said(&ended),
        [
            "The copy was made, but the original was already gone from the server: entity not found",
            "The folder was copied, but it was already gone from the server: entity not found",
        ]
    );
    assert_eq!(read(&local.path().join("a.txt")), b"a");
}

#[test]
fn a_file_that_changed_while_it_was_read_or_after_is_kept() {
    for grow in [Grow::AtEnd, Grow::OnClose] {
        let server = Fake::new();
        server.put("log.txt", b"first line\n");
        server.switch(|switches| switches.grow = grow);
        let local = temp();

        let ended = fetch(&mut Rig::new(), OpKind::Move, &server, &["log.txt"], local.path());

        assert_eq!(said(&ended), [KEPT]);
        assert_eq!(read(&server.at("log.txt")), b"first line\nmore");
        assert_eq!(read(&local.path().join("log.txt")), b"first line\n");
    }
}

#[test]
fn a_server_that_gives_no_time_keeps_its_files_and_nothing_is_copied() {
    let server = Fake::new();
    server.put("a.txt", b"a");
    server.switch(|switches| switches.no_mtime = true);
    let local = temp();

    let ended = fetch(&mut Rig::new(), OpKind::Move, &server, &["a.txt"], local.path());

    assert_eq!(
        said(&ended),
        ["Kept the original: the server does not tell when it was changed"]
    );
    assert_eq!(tree(&server.at("")), ["a.txt"]);
    assert_eq!(tree(local.path()), Vec::<String>::new());

    let copied = fetch(&mut Rig::new(), OpKind::Copy, &server, &["a.txt"], local.path());

    assert_eq!(copied.errors.len(), 0, "a copy needs no time");
    assert_eq!(read(&local.path().join("a.txt")), b"a");
}

#[test]
fn a_copy_that_cannot_be_flushed_keeps_the_original_and_commits_nothing() {
    let server = Fake::new();
    server.put("a.bin", &pattern(100_000));
    let local = temp();
    let mut rig = Rig::hooked(|hooks| hooks.fail_flush = true);

    let ended = fetch(&mut rig, OpKind::Move, &server, &["a.bin"], local.path());

    assert_eq!(ended.errors.len(), 1, "{:?}", ended.errors);
    assert_eq!(tree(&server.at("")), ["a.bin"]);
    assert_eq!(tree(local.path()), Vec::<String>::new());
    assert!(has_no_partial_files(local.path()));
}

#[test]
fn a_copy_that_lost_its_name_to_another_file_keeps_the_original() {
    let server = Fake::new();
    server.put("a.txt", b"from the server");
    let local = temp();
    claim_meanwhile(&server, &local.path().join("a.txt"), b"made meanwhile");
    let request = download(OpKind::Move, &server, "/", &["a.txt"], local.path());

    let (_, ended) = answered(&mut Rig::new(), request, Answer::Skip);

    assert_eq!(ended.skipped, 1);
    assert_eq!(tree(&server.at("")), ["a.txt"]);
    assert_eq!(read(&local.path().join("a.txt")), b"made meanwhile");
}
