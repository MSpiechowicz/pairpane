//! A download that goes wrong or is stopped: what the server sends that is not what it said, a connection that fails, a
//! cancel, a panic. Whatever happens, no temporary file, no probe and no open handle is left.

use std::io::ErrorKind;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pretty_assertions::assert_eq;

use crate::model::{ControlAction, OpKind, Outcome};

use super::super::testing::{Rig, pattern, read, temp, tree};
use super::testing::{Fake, download, failure, fetch, said};

#[test]
fn a_file_larger_or_smaller_than_the_server_said_is_not_kept() {
    for (skew, message) in [
        (-3, "the file is larger than its size on the server"),
        (5, "the file ended before its size on the server"),
    ] {
        let server = Fake::new();
        server.put("a.bin", &pattern(1_000));
        server.switch(|switches| switches.size_skew = skew);
        let local = temp();

        let ended = fetch(&mut Rig::new(), OpKind::Move, &server, &["a.bin"], local.path());

        assert_eq!(said(&ended), [message]);
        assert_eq!(tree(local.path()), Vec::<String>::new());
        assert_eq!(tree(&server.at("")), ["a.bin"], "the original stays");
        assert_eq!(server.handles(), 0);
    }
}

#[test]
fn a_connection_lost_in_the_middle_of_a_file_fails_that_item_only() {
    let server = Fake::new();
    server.put("big.bin", &pattern(600_000));
    server.put("small.txt", b"small");
    server.switch(|switches| switches.fail_after = Some(300_000));
    let local = temp();

    let ended = fetch(
        &mut Rig::new(),
        OpKind::Copy,
        &server,
        &["big.bin", "small.txt"],
        local.path(),
    );

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 1);
    assert_eq!(ended.errors[0].path, "big.bin");
    assert_eq!(tree(local.path()), ["small.txt"], "no half file, no temporary file");
    assert_eq!(server.handles(), 0);
}

#[test]
fn a_server_that_is_gone_fails_the_job() {
    let server = Fake::new();
    server.put("a.txt", b"a");
    server.put("b.txt", b"b");
    server.switch(|switches| switches.open_errors = [Some(ErrorKind::NotConnected)].into());
    let local = temp();

    let ended = fetch(
        &mut Rig::new(),
        OpKind::Copy,
        &server,
        &["a.txt", "b.txt"],
        local.path(),
    );

    assert!(matches!(ended.outcome, Outcome::Failed { .. }));
    assert_eq!(tree(local.path()), Vec::<String>::new(), "the next item is not tried");
}

#[test]
fn three_items_in_a_row_that_time_out_or_are_refused_a_connection_fail_the_job() {
    for kind in [ErrorKind::TimedOut, ErrorKind::ConnectionRefused] {
        let server = Fake::new();
        for name in ["a", "b", "c", "d"] {
            server.put(name, name.as_bytes());
        }
        server.switch(|switches| switches.open_errors = [Some(kind); 3].into());
        let local = temp();

        let ended = fetch(
            &mut Rig::new(),
            OpKind::Copy,
            &server,
            &["a", "b", "c", "d"],
            local.path(),
        );

        assert_eq!(failure(&ended), "the server stopped answering", "{kind:?}");
        assert_eq!(ended.errors.len(), 3, "{kind:?}");
        assert_eq!(tree(local.path()), Vec::<String>::new(), "{kind:?}");
    }
}

#[test]
fn an_answer_between_the_failures_starts_the_count_again() {
    let server = Fake::new();
    for name in ["a", "b", "c", "d"] {
        server.put(name, name.as_bytes());
    }
    let timed_out = Some(ErrorKind::ConnectionAborted);
    server.switch(|switches| switches.open_errors = [timed_out, timed_out, None, timed_out].into());
    let local = temp();

    let ended = fetch(
        &mut Rig::new(),
        OpKind::Copy,
        &server,
        &["a", "b", "c", "d"],
        local.path(),
    );

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 3);
    assert_eq!(
        tree(local.path()),
        ["c"],
        "the fourth fails alone: the count began again at the third"
    );
}

#[test]
fn a_cancel_stops_a_read_that_never_answers_and_leaves_nothing() {
    let server = Fake::new();
    server.put("d/big.bin", &pattern(1_000_000));
    let (sender, started) = mpsc::channel();
    server.switch(|switches| {
        switches.hang_reads = true;
        switches.at_first_read = Some(std::sync::Arc::new(move || sender.send(()).expect("listening")));
    });
    let local = temp();
    let mut rig = Rig::new();

    let job = rig.start(download(OpKind::Copy, &server, "/", &["d"], local.path()));
    started.recv_timeout(Duration::from_secs(10)).expect("the read started");
    let cancelled_at = Instant::now();
    rig.engine.control(job, ControlAction::Cancel).expect("known job");
    let ended = rig.wait_ended(job);

    assert_eq!(ended.outcome, Outcome::Cancelled);
    assert!(
        cancelled_at.elapsed() < Duration::from_secs(2),
        "the read was interrupted"
    );
    assert_eq!(
        tree(local.path()),
        Vec::<String>::new(),
        "no temporary file, no probe, no folder"
    );
    assert_eq!(server.handles(), 0);
}

#[test]
fn a_panic_in_the_middle_of_a_file_leaves_nothing() {
    let server = Fake::new();
    server.put("d/a.bin", &pattern(1_000));
    server.switch(|switches| switches.panic_reads = true);
    let local = temp();

    let ended = fetch(&mut Rig::new(), OpKind::Copy, &server, &["d"], local.path());

    assert!(matches!(ended.outcome, Outcome::Failed { .. }), "{:?}", ended.outcome);
    assert_eq!(tree(local.path()), Vec::<String>::new());
    assert_eq!(server.handles(), 0);
    assert_eq!(read(&server.at("d/a.bin")), pattern(1_000));
}
