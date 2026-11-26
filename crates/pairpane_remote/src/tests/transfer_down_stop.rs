//! A download that goes wrong or is stopped, over real SFTP: a server that does not answer, a connection that is lost,
//! a site that is closed. Whatever happens, no temporary file, no probe and no open handle is left on this computer.

use std::fs;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use pairpane_core::model::{ControlAction, JobId, OpKind, Outcome};
use pretty_assertions::assert_eq;

use super::down_rig::{Rig, tree};
use super::transfer_fs::pattern;
use crate::test_server::Settings;

/// A server that stops answering READ on a connection once it has had ten.
fn stalling() -> Settings {
    Settings {
        stall_reads_after: Some(10),
        ..Settings::default()
    }
}

#[test]
fn a_cancel_ends_a_download_the_server_does_not_answer_and_leaves_nothing() {
    let mut rig = Rig::new(Settings {
        stall_reads_after: Some(0),
        ..Settings::default()
    });

    rig.h.server.put("data/stuck.bin", &pattern(1_000_000));

    let job = rig.start(rig.request(OpKind::Copy, "/data", &["stuck.bin"]));

    rig.wait_for_reads(1);

    let started = Instant::now();

    rig.control(job, ControlAction::Cancel);

    let ended = rig.ended(job);

    assert_eq!(ended.outcome, Outcome::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    assert_eq!(
        rig.local_tree(),
        Vec::<String>::new(),
        "no partial file and no probe is left"
    );
    assert!(rig.on_server("data/stuck.bin").exists());

    // The server answers again: what it was asked is answered, and then the handle is closed.
    rig.h.server.counts.release_reads();
    rig.no_handles_left();
}

/// A move of a big file and a small one, started on a server that stops answering READ: the job is in the big file, with
/// requests held back, when this returns.
fn stuck_in_the_first_of_two(rig: &mut Rig) -> JobId {
    rig.h.server.put("data/big.bin", &pattern(3_000_000));
    rig.h.server.put("data/next.txt", b"next");

    let job = rig.start(rig.request(OpKind::Move, "/data", &["big.bin", "next.txt"]));

    rig.wait_for_reads(11);

    job
}

#[test]
fn a_connection_lost_in_the_middle_of_a_file_fails_that_item_and_the_next_one_reconnects() {
    let mut rig = Rig::new(stalling());
    let job = stuck_in_the_first_of_two(&mut rig);

    rig.h.server.drop_connections();

    let ended = rig.ended(job);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.failed(), ["big.bin"]);
    assert_eq!(rig.local_tree(), ["next.txt"], "no half file, no temporary file");
    assert_eq!(fs::read(rig.local.path().join("next.txt")).expect("file"), b"next");
    assert_eq!(
        tree(&rig.on_server("data")),
        ["big.bin"],
        "the one that failed stays on the server, the other is moved"
    );
    assert_eq!(rig.h.server.counts.connections.load(Ordering::Relaxed), 2);
    rig.no_handles_left();
}

#[test]
fn a_site_closed_in_the_middle_of_a_job_fails_it_promptly() {
    let mut rig = Rig::new(stalling());
    let job = stuck_in_the_first_of_two(&mut rig);
    let started = Instant::now();

    rig.h.pool.disconnect(rig.site);

    let ended = rig.ended(job);

    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    assert!(matches!(ended.outcome, Outcome::Failed { .. }), "{ended:?}");
    assert_eq!(rig.local_tree(), Vec::<String>::new());
    assert_eq!(
        tree(&rig.on_server("data")),
        ["big.bin", "next.txt"],
        "nothing was removed"
    );
    assert_eq!(rig.h.server.counts.removes.load(Ordering::Relaxed), 0);
    rig.no_handles_left();
}
