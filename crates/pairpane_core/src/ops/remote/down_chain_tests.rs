//! A move through a long way of folders, and a move that is stopped or cannot tell what is left: the check of each
//! removal costs about one look at the server however deep the folder is, a cancel is heard during the removals, and a
//! folder that cannot be told empty is reported, not kept without a word.

use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::model::{ControlAction, OpKind, Outcome};

use super::super::testing::{Ended, Rig, temp, tree};
use super::testing::{Fake, download, fetch, said};

/// The folders of the longest way a job accepts.
#[cfg(target_os = "linux")]
const DEPTH: usize = 1_023;

// Linux only: a path of a thousand names is longer than what macOS and Windows accept.
#[cfg(target_os = "linux")]
#[test]
fn the_looks_of_a_move_through_the_deepest_tree_a_job_accepts_do_not_grow_with_the_square_of_its_depth() {
    let server = Fake::new();
    let way = "a/".repeat(DEPTH);

    server.put(&format!("{way}f.txt"), b"x");
    // The cost under test is of the looks at the way, not of a server that resolves links one by one.
    server.switch(|switches| switches.lexical_realpath = true);
    let local = temp();

    let ended = fetch(&mut Rig::new(), OpKind::Move, &server, &["a"], local.path());

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 0, "{:?}", said(&ended));
    assert_eq!(
        tree(&server.at("")),
        Vec::<String>::new(),
        "every file and folder was moved"
    );
    // A look at each name of the way, one after the other, is a million calls: every removal has a way to look at.
    assert!(
        server.round_trips() < 20 * DEPTH,
        "{} calls waited for one after the other",
        server.round_trips()
    );
}

/// What a server does at a moment a test chooses: it says so on the first channel, and waits for the second.
pub(super) fn gate() -> (Arc<dyn Fn() + Send + Sync>, mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (reached, stopped) = mpsc::channel();
    let (go, go_on) = mpsc::channel::<()>();
    let go_on = Mutex::new(go_on);
    let hook = Arc::new(move || {
        reached.send(()).expect("listening");
        go_on
            .lock()
            .expect("one reader")
            .recv_timeout(Duration::from_secs(10))
            .expect("told to go on");
    });

    (hook, stopped, go)
}

/// Moves `item` from the root of `server`, whose hook `stopped` reports (and `go` releases), and cancels the job while the
/// server is held.
pub(super) fn cancelled_at(
    server: &Arc<Fake>,
    item: &str,
    stopped: &mpsc::Receiver<()>,
    go: &mpsc::Sender<()>,
) -> (Ended, TempDir) {
    let local = temp();
    let mut rig = Rig::new();

    let job = rig.start(download(OpKind::Move, server, "/", &[item], local.path()));
    stopped
        .recv_timeout(Duration::from_secs(10))
        .expect("the server reached the moment");
    rig.engine.control(job, ControlAction::Cancel).expect("known job");
    go.send(()).expect("the server waits");

    (rig.wait_ended(job), local)
}

#[test]
fn a_cancel_during_the_removals_ends_the_job_cancelled_with_the_folders_kept() {
    let server = Fake::new();
    let (hook, stopped, go) = gate();

    server.put("d/e/a.txt", b"a");
    server.switch(|switches| switches.after_remove = Some(hook));

    let (ended, local) = cancelled_at(&server, "d", &stopped, &go);

    assert_eq!(ended.outcome, Outcome::Cancelled);
    assert_eq!(said(&ended), Vec::<String>::new(), "nothing went wrong");
    assert_eq!(
        tree(&server.at("")),
        ["d/", "d/e/"],
        "no folder is removed once cancelled"
    );
    assert_eq!(
        tree(local.path()),
        ["d/", "d/e/", "d/e/a.txt"],
        "the copy that was made stays"
    );
}

#[test]
fn a_cancel_that_comes_while_a_folder_is_looked_at_for_what_is_left_in_it_ends_the_job_cancelled() {
    let server = Fake::new();
    let (hook, stopped, go) = gate();

    server.put("d/a.txt", b"a");
    // The scan lists the folder, then the copy, and then the move looks at what is left of it: there it is cancelled.
    server.switch(|switches| switches.at_listing = Some((3, hook)));

    let (ended, _local) = cancelled_at(&server, "d", &stopped, &go);

    assert_eq!(ended.outcome, Outcome::Cancelled);
    assert_eq!(tree(&server.at("")), ["d/"], "the file is moved, the folder is kept");
}

#[test]
fn a_folder_that_cannot_be_told_empty_is_reported_and_kept() {
    let server = Fake::new();
    server.put("d/a.txt", b"a");
    // The scan lists the folder, then the copy, and then the move looks at what is left of it.
    server.switch(|switches| switches.fail_listings_from = Some(3));
    let local = temp();

    let ended = fetch(&mut Rig::new(), OpKind::Move, &server, &["d"], local.path());

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(
        said(&ended),
        [
            "The folder was copied, but what is left in it could not be listed, so it stays on the server: the server could not list the folder"
        ]
    );
    assert_eq!(tree(&server.at("")), ["d/"], "the file is moved, the folder is kept");
}
