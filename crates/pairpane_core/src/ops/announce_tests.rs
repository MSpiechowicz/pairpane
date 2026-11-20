//! What the listener is told about a job from the threads that control it: never anything after its end.

use pretty_assertions::assert_eq;

use crate::model::{ControlAction, OpsEvent};

use super::testing::{Rig, put, temp};

#[test]
fn nothing_is_announced_about_a_job_after_it_has_ended() {
    let root = temp();
    put(root.path(), "src/a.txt", b"a");
    let mut rig = Rig::paused_at_start();
    let job = rig.copy_in(root.path(), &["a.txt"]);
    rig.wait(|event| matches!(event, OpsEvent::Paused { paused: true, .. }));

    // A thread that looked the job up before it ended, and acts on it after.
    let stale = rig.engine.inner.job(job).expect("the job is running");
    rig.engine.control(job, ControlAction::Resume).expect("known job");
    rig.wait_ended(job);
    rig.drain();
    let before = rig.seen.len();

    stale.control.pause();
    rig.engine.inner.announce_paused(&stale);
    rig.engine.inner.announce_stopping(std::slice::from_ref(&stale));
    rig.drain();

    assert_eq!(rig.seen.len(), before, "{:#?}", &rig.seen[before..]);
    assert_eq!(rig.ended_count(job), 1);
}
