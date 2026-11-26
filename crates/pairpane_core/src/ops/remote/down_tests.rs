//! A copy from a server into a folder of this computer: what lands, what it keeps, what it asks, what it leaves alone.

use std::fs;

use pretty_assertions::assert_eq;

use crate::model::{Answer, OpKind, Outcome};
use crate::vfs::RemoteKind;

use super::super::testing::{Rig, has_no_partial_files, long_ago, modified, pattern, read, set_times, temp, tree};
use super::testing::{Fake, answered, claim_meanwhile, download, failure, fetch, said};

#[test]
fn a_tree_is_copied_whole_with_the_times_of_the_server() {
    let server = Fake::new();
    server.put("docs/a.txt", b"alpha");
    server.put("docs/deep/b.bin", &pattern(700_000));
    server.put("c.txt", b"c");
    set_times(&server.at("docs/a.txt"), long_ago());
    set_times(&server.at("docs/deep"), long_ago());
    set_times(&server.at("docs"), long_ago());
    let local = temp();

    let ended = fetch(&mut Rig::new(), OpKind::Copy, &server, &["docs", "c.txt"], local.path());

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!((ended.skipped, ended.errors.len()), (0, 0), "{:?}", ended.errors);
    assert_eq!(
        tree(local.path()),
        ["c.txt", "docs/", "docs/a.txt", "docs/deep/", "docs/deep/b.bin"]
    );
    assert_eq!(read(&local.path().join("docs/a.txt")), b"alpha");
    assert_eq!(read(&local.path().join("docs/deep/b.bin")), pattern(700_000));
    assert_eq!(modified(&local.path().join("docs/a.txt")), long_ago());
    assert_eq!(
        modified(&local.path().join("docs")),
        long_ago(),
        "a folder is given its time last"
    );
    assert_eq!(tree(&server.at("")).len(), 5, "a copy leaves the server as it was");
    assert_eq!(server.handles(), 0);
}

#[cfg(unix)]
#[test]
fn the_mode_follows_the_rule_of_an_extraction() {
    use std::os::unix::fs::PermissionsExt;

    let server = Fake::new();
    server.put("run.sh", b"program");
    server.put("open.txt", b"x");
    fs::set_permissions(server.at("run.sh"), fs::Permissions::from_mode(0o755)).expect("mode");
    fs::set_permissions(server.at("open.txt"), fs::Permissions::from_mode(0o666)).expect("mode");
    let local = temp();

    fetch(
        &mut Rig::new(),
        OpKind::Copy,
        &server,
        &["run.sh", "open.txt"],
        local.path(),
    );

    let mode = |name: &str| {
        fs::metadata(local.path().join(name))
            .expect("file")
            .permissions()
            .mode()
            & 0o7777
    };
    assert_eq!(mode("run.sh"), 0o755);
    assert_eq!(mode("open.txt"), 0o644, "no write for the group and others");
}

#[test]
fn reads_in_pieces_and_in_packets_give_the_same_file() {
    let server = Fake::new();
    server.put("small.bin", &pattern(100_000));
    server.put("big.bin", &pattern(1_300_000));
    server.switch(|switches| switches.short_reads = Some(7_001));
    let local = temp();

    let ended = fetch(
        &mut Rig::new(),
        OpKind::Copy,
        &server,
        &["small.bin", "big.bin"],
        local.path(),
    );

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(read(&local.path().join("small.bin")), pattern(100_000));
    assert_eq!(read(&local.path().join("big.bin")), pattern(1_300_000));
}

#[test]
fn no_read_asks_for_more_than_one_packet() {
    let server = Fake::new();
    server.put("big.bin", &pattern(1_300_000));
    let local = temp();

    fetch(&mut Rig::new(), OpKind::Copy, &server, &["big.bin"], local.path());

    assert_eq!(
        server.biggest_read(),
        256 * 1024,
        "a pause or a cancel is noticed between packets"
    );
}

#[test]
fn a_target_that_exists_is_asked_about_without_a_compare_and_kept_on_skip() {
    let server = Fake::new();
    server.put("a.txt", b"new");
    let local = temp();
    fs::write(local.path().join("a.txt"), b"old").expect("file");
    let request = download(OpKind::Copy, &server, "/", &["a.txt"], local.path());

    let (question, ended) = answered(&mut Rig::new(), request, Answer::Skip);

    assert!(!question.comparable, "the server's file cannot be compared");
    assert_eq!(question.source.size, 3);
    assert_eq!((ended.skipped, ended.errors.len()), (1, 0));
    assert_eq!(read(&local.path().join("a.txt")), b"old");
    assert_eq!(tree(local.path()), ["a.txt"]);
}

#[test]
fn overwrite_replaces_and_rename_keeps_both() {
    let server = Fake::new();
    server.put("a.txt", b"new");
    let local = temp();
    fs::write(local.path().join("a.txt"), b"old").expect("file");
    let mut rig = Rig::new();
    let request = download(OpKind::Copy, &server, "/", &["a.txt"], local.path());

    answered(&mut rig, request.clone(), Answer::Rename);
    answered(&mut rig, request, Answer::Overwrite);

    assert_eq!(read(&local.path().join("a.txt")), b"new");
    assert_eq!(read(&local.path().join("a (2).txt")), b"new");
}

#[test]
fn a_target_that_appears_while_the_file_is_written_is_not_overwritten() {
    let server = Fake::new();
    server.put("a.txt", b"from the server");
    let local = temp();
    claim_meanwhile(&server, &local.path().join("a.txt"), b"made meanwhile");
    let request = download(OpKind::Copy, &server, "/", &["a.txt"], local.path());

    let (_, ended) = answered(&mut Rig::new(), request, Answer::Skip);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(read(&local.path().join("a.txt")), b"made meanwhile");
    assert_eq!(tree(local.path()), ["a.txt"], "the temporary file is gone");
}

#[test]
fn a_verified_copy_is_checked_and_one_that_differs_is_not_kept() {
    let server = Fake::new();
    server.put("a.bin", &pattern(300_000));
    let local = temp();
    let mut request = download(OpKind::Copy, &server, "/", &["a.bin"], local.path());
    request.verify = true;

    let ended = Rig::new().run(request.clone());
    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(read(&local.path().join("a.bin")), pattern(300_000));

    fs::remove_file(local.path().join("a.bin")).expect("removed");
    let ended = Rig::hooked(|hooks| hooks.corrupt_verify = true).run(request);

    assert_eq!(said(&ended), ["the copy does not match the original"]);
    assert_eq!(tree(local.path()), Vec::<String>::new());
}

#[cfg(unix)]
#[test]
fn links_and_special_files_and_unusable_names_are_skipped_and_counted() {
    let server = Fake::new();
    server.put("d/real.txt", b"real");
    std::os::unix::fs::symlink("real.txt", server.at("d/link")).expect("link");
    std::os::unix::fs::symlink("d", server.at("toplink")).expect("link");
    super::super::testing::make_fifo(&server.at("d/pipe"));
    server.switch(|switches| {
        switches.extra_entries = vec![("a/b".to_owned(), RemoteKind::File), ("..".to_owned(), RemoteKind::Dir)];
    });
    let local = temp();

    let ended = fetch(&mut Rig::new(), OpKind::Copy, &server, &["d", "toplink"], local.path());

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 0, "{:?}", ended.errors);
    assert_eq!(
        ended.skipped, 4,
        "the link, the pipe, the name with a slash and the link of the job"
    );
    assert_eq!(tree(local.path()), ["d/", "d/real.txt"], "a link is never followed");
}

#[test]
fn a_file_that_is_gone_is_reported_and_the_others_go_on() {
    let server = Fake::new();
    server.put("here.txt", b"x");
    let local = temp();

    let ended = fetch(
        &mut Rig::new(),
        OpKind::Copy,
        &server,
        &["gone.txt", "here.txt"],
        local.path(),
    );

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors[0].path, "gone.txt");
    assert_eq!(tree(local.path()), ["here.txt"]);
}

#[test]
fn the_same_folder_is_refused_before_anything_is_written() {
    for kind in [OpKind::Copy, OpKind::Move] {
        let local = temp();
        fs::write(local.path().join("a.txt"), b"mine").expect("file");
        let server = Fake::over(local.path());

        let ended = fetch(&mut Rig::new(), kind, &server, &["a.txt"], local.path());

        assert_eq!(failure(&ended), "the folder of the items is the target folder");
        assert_eq!(
            tree(local.path()),
            ["a.txt"],
            "no probe is left and nothing was touched"
        );
        assert_eq!(read(&local.path().join("a.txt")), b"mine");
    }
}

#[test]
fn a_folder_is_not_copied_into_itself_and_the_folder_made_for_it_is_removed() {
    // Also when the listing shows files as items that are not copied: the file that tells the target is still seen.
    for files_as_other in [false, true] {
        let local = temp();
        fs::create_dir(local.path().join("d")).expect("folder");
        fs::write(local.path().join("d/x.txt"), b"x").expect("file");
        let server = Fake::over(local.path());
        server.switch(|switches| switches.files_as_other = files_as_other);

        let ended = fetch(
            &mut Rig::new(),
            OpKind::Move,
            &server,
            &["d"],
            &local.path().join("d/in"),
        );

        assert_eq!(
            failure(&ended),
            "a folder cannot be copied or moved into itself",
            "{files_as_other}"
        );
        assert_eq!(tree(local.path()), ["d/", "d/x.txt"], "{files_as_other}");
    }
}

#[test]
fn a_cancelled_job_before_it_starts_leaves_no_trace() {
    let server = Fake::new();
    server.put("a.txt", b"x");
    let local = temp();
    let mut rig = Rig::paused_at_start();

    let job = rig.start(download(OpKind::Copy, &server, "/", &["a.txt"], local.path()));
    let ended = rig.cancel_when_paused(job);

    assert_eq!(ended.outcome, Outcome::Cancelled);
    assert!(has_no_partial_files(local.path()));
    assert_eq!(tree(local.path()), Vec::<String>::new());
}
