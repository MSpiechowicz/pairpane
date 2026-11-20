//! The queue, the workers and the life of a job: order per volume, the caps, pause, cancel, and the events.

use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use pretty_assertions::assert_eq;

use crate::model::{Answer, ControlAction, IpcErrorCode, JobId, JobPhase, OpsEvent, Outcome};

use super::testing::{
    Rig, blocked, blocked_scene, copy, phases, put, read, temp, tree, view_of, volume_by_name, write,
};
use super::volume::VolumeKey;
use super::{MAX_JOBS, MAX_NAME_UNITS, MAX_NAMES, MAX_NAMES_BYTES, MAX_WORKERS, Refused, UnknownJob};

fn position(rig: &Rig, wanted: impl Fn(&OpsEvent) -> bool) -> usize {
    rig.seen.iter().position(wanted).expect("the event was seen")
}

#[test]
fn a_job_announces_itself_first_goes_through_its_phases_and_ends_once() {
    let root = temp();
    put(root.path(), "src/a.txt", b"hello");
    let mut rig = Rig::new();

    let job = rig.copy_in(root.path(), &["a.txt"]);
    let ended = rig.wait_ended(job);
    rig.drain();

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(read(&root.path().join("dst/a.txt")), b"hello");
    assert!(matches!(&rig.seen[0], OpsEvent::Added { job: view } if view.id == job && view.phase == JobPhase::Queued));
    assert_eq!(
        phases(&rig),
        [JobPhase::Preparing, JobPhase::Scanning, JobPhase::Copying]
    );
    assert_eq!(rig.ended_count(job), 1);
    assert!(
        matches!(rig.seen.last(), Some(OpsEvent::Ended { .. })),
        "{:#?}",
        rig.seen
    );
    assert_eq!(
        rig.engine.snapshot(),
        OpsEvent::Snapshot {
            jobs: vec![],
            questions: vec![]
        }
    );
}

#[test]
fn submit_refuses_what_needs_no_disk_to_refuse() {
    let rig = Rig::new();
    let (from, to) = (Path::new("/from"), Path::new("/to"));

    let too_many: Vec<String> = (0..=MAX_NAMES).map(|index| format!("n{index}")).collect();
    let mut request = copy(from, &[], to);
    assert_eq!(rig.engine.submit(request.clone()), Err(Refused::InvalidNames));

    request.names = too_many;
    assert_eq!(rig.engine.submit(request), Err(Refused::InvalidNames));
    assert_eq!(
        rig.engine.submit(copy(from, &["a", "a"], to)),
        Err(Refused::InvalidNames)
    );

    for bad in ["", ".", "..", "a/b"] {
        assert_eq!(
            rig.engine.submit(copy(from, &[bad], to)),
            Err(Refused::InvalidNames),
            "{bad:?}"
        );
    }
    assert_eq!(
        rig.engine.snapshot(),
        OpsEvent::Snapshot {
            jobs: vec![],
            questions: vec![]
        }
    );
}

#[test]
fn submit_caps_the_size_of_a_name_and_of_all_the_names() {
    let rig = Rig::new();
    let (from, to) = (Path::new("/from"), Path::new("/to"));
    let units = |count: usize| "a".repeat(count);

    // The longest name passes the check (and the job then fails on its missing folder); one unit more is refused.
    let longest = units(MAX_NAME_UNITS);
    let ok = rig.engine.submit(copy(from, &[longest.as_str()], to));
    assert!(ok.is_ok(), "{ok:?}");
    assert_eq!(
        rig.engine.submit(copy(from, &[units(MAX_NAME_UNITS + 1).as_str()], to)),
        Err(Refused::NamesTooLarge)
    );

    // Many names, each short enough, whose total is over the cap.
    let per_name = MAX_NAME_UNITS;
    let count = MAX_NAMES_BYTES / per_name + 1;
    let names: Vec<String> = (0..count).map(|index| format!("{index:0>per_name$}")).collect();
    assert!(names.len() <= MAX_NAMES);
    let mut request = copy(from, &[], to);
    request.names = names;
    assert_eq!(rig.engine.submit(request), Err(Refused::NamesTooLarge));
    assert_eq!(
        pairpane_error(Refused::NamesTooLarge),
        IpcErrorCode::InvalidInput,
        "reported as invalid input"
    );
}

fn pairpane_error(refused: Refused) -> IpcErrorCode {
    crate::model::IpcError::from(refused).code
}

#[test]
fn a_missing_source_folder_fails_the_job_and_not_the_submit() {
    let root = temp();
    let mut rig = Rig::new();

    let ended = rig.run(copy(&root.path().join("missing"), &["a"], &root.path().join("dst")));

    let Outcome::Failed { error } = ended.outcome else {
        panic!("{:?}", ended.outcome);
    };
    assert_eq!(error.code, IpcErrorCode::NotFound);
    assert!(!root.path().join("dst").exists());
}

#[test]
fn paths_that_are_not_absolute_or_climb_fail_the_job_before_anything_is_touched() {
    let root = temp();
    write(&root.path().join("src/a.txt"), b"x");
    let mut rig = Rig::new();
    let climbing = root.path().join("src").join("..").join("elsewhere");

    for (from, to) in [
        (Path::new("relative").to_owned(), root.path().join("dst")),
        (root.path().join("src"), Path::new("relative/dst").to_owned()),
        (root.path().join("src"), climbing),
    ] {
        let ended = rig.run(copy(&from, &["a.txt"], &to));

        let Outcome::Failed { error } = ended.outcome else {
            panic!("{from:?} -> {to:?}: {:?}", ended.outcome);
        };
        assert_eq!(error.code, IpcErrorCode::InvalidInput, "{from:?} -> {to:?}");
    }
    assert_eq!(tree(root.path()), ["src/", "src/a.txt"]);
}

#[cfg(windows)]
#[test]
fn network_and_device_paths_fail_the_job_before_anything_is_opened() {
    let root = temp();
    write(&root.path().join("src/a.txt"), b"x");
    let mut rig = Rig::new();

    for remote in [r"\\attacker\share\dst", r"\\?\UNC\attacker\share\dst", r"\\.\pipe\x"] {
        let to_remote = rig.run(copy(&root.path().join("src"), &["a.txt"], Path::new(remote)));
        let from_remote = rig.run(copy(Path::new(remote), &["a.txt"], &root.path().join("dst")));

        for ended in [to_remote, from_remote] {
            assert!(
                matches!(ended.outcome, Outcome::Failed { .. }),
                "{remote}: {:?}",
                ended.outcome
            );
        }
    }
}

#[test]
fn jobs_of_one_volume_run_in_submit_order_and_other_volumes_run_alongside() {
    let (root, src) = blocked_scene();
    let mut rig = Rig::new_by_name();

    let first = blocked(&rig, &src, &root.path().join("same-a"));
    let question = rig.wait_conflict(first);
    let second = rig.start(copy(&src, &["f.txt"], &root.path().join("same-b")));
    let other = rig.start(copy(&src, &["f.txt"], &root.path().join("other-c")));

    // The other volume finishes while the first job still waits, and the second waits its turn.
    assert_eq!(rig.wait_ended(other).outcome, Outcome::Done);
    assert_eq!(view_of(&rig.engine, first).phase, JobPhase::Waiting);
    assert_eq!(view_of(&rig.engine, second).phase, JobPhase::Queued);

    rig.engine
        .answer(first, question.question, Answer::Skip, false)
        .expect("open question");
    assert_eq!(rig.wait_ended(first).outcome, Outcome::Done);
    assert_eq!(rig.wait_ended(second).outcome, Outcome::Done);

    let first_ended = position(
        &rig,
        |event| matches!(event, OpsEvent::Ended { id, .. } if *id == first),
    );
    let second_started = position(
        &rig,
        |event| matches!(event, OpsEvent::Phase { id, .. } if *id == second),
    );
    assert!(
        first_ended < second_started,
        "the second job started before the first ended"
    );
    assert_eq!(read(&root.path().join("same-b/f.txt")), b"new");
}

#[test]
fn no_more_than_four_workers_run_at_once() {
    let (root, src) = blocked_scene();
    let mut rig = Rig::new_by_name();
    assert_eq!(MAX_WORKERS, 4);

    let jobs: Vec<JobId> = (0..5)
        .map(|index| blocked(&rig, &src, &root.path().join(format!("v{index}-x"))))
        .collect();
    let questions: Vec<_> = jobs[..4].iter().map(|job| rig.wait_conflict(*job)).collect();

    let phases: Vec<JobPhase> = jobs.iter().map(|job| view_of(&rig.engine, *job).phase).collect();
    assert_eq!(phases[..4], [JobPhase::Waiting; 4]);
    assert_eq!(phases[4], JobPhase::Queued);

    // A worker that finishes takes the fifth job.
    rig.engine
        .answer(jobs[0], questions[0].question, Answer::Skip, false)
        .expect("open question");
    let fifth = rig.wait_conflict(jobs[4]);
    assert_eq!(view_of(&rig.engine, jobs[4]).phase, JobPhase::Waiting);

    rig.engine.cancel_all();
    for job in &jobs {
        rig.wait_ended(*job);
    }
    assert_eq!(fifth.job, jobs[4]);
}

/// A volume lookup that is stuck: `entered` hears when it starts, and it ends when `open_gate` is used.
struct Hang {
    open_gate: mpsc::Sender<()>,
    entered: mpsc::Receiver<()>,
}

/// A rig whose volume lookup for a target ending in `v0-x` hangs; the others answer at once.
fn hung_first_lookup() -> (Rig, Hang) {
    let (open_gate, gate) = mpsc::channel::<()>();
    let (entering, entered) = mpsc::channel::<()>();
    let gate = Mutex::new(gate);
    let slow_first = move |target: &Path| {
        if target.ends_with("v0-x") {
            let _ = entering.send(());
            let _ = gate.lock().expect("gate").recv();
        }

        volume_by_name(target)
    };

    (
        Rig::hooked(|hooks| hooks.keyer = Some(Arc::new(slow_first))),
        Hang { open_gate, entered },
    )
}

impl Hang {
    fn wait_until_stuck(&self) {
        self.entered
            .recv_timeout(Duration::from_secs(10))
            .expect("the lookup started");
    }
}

#[test]
fn a_volume_lookup_that_hangs_holds_only_its_own_worker() {
    let (root, src) = blocked_scene();
    let (mut rig, hang) = hung_first_lookup();

    let first = blocked(&rig, &src, &root.path().join("v0-x"));
    hang.wait_until_stuck();
    let others: Vec<JobId> = (1..3)
        .map(|index| blocked(&rig, &src, &root.path().join(format!("v{index}-x"))))
        .collect();

    // The jobs on other volumes run, one worker each, while the first lookup is still going on.
    for job in &others {
        rig.wait_conflict(*job);
    }
    assert_eq!(view_of(&rig.engine, first).phase, JobPhase::Queued);
    // At least the stuck lookup's worker and one per running job (a spare one may be on its way out).
    assert!(rig.engine.workers() >= 3);

    hang.open_gate.send(()).expect("the lookup waits");
    rig.wait_conflict(first);
    rig.engine.cancel_all();
}

#[test]
fn a_job_whose_lookup_hangs_is_cancelled_without_waiting_for_it() {
    let (root, src) = blocked_scene();
    let (mut rig, hang) = hung_first_lookup();

    let first = blocked(&rig, &src, &root.path().join("v0-x"));
    hang.wait_until_stuck();
    let second = blocked(&rig, &src, &root.path().join("v1-x"));
    rig.wait_conflict(second);

    rig.engine.control(first, ControlAction::Cancel).expect("known job");

    // Ended while the lookup is still stuck.
    assert_eq!(rig.wait_ended(first).outcome, Outcome::Cancelled);
    assert_eq!(rig.engine.control(first, ControlAction::Cancel), Err(UnknownJob));
    assert_eq!(view_of(&rig.engine, second).phase, JobPhase::Waiting);

    // The lookup comes back to a job that is gone, and its worker ends.
    hang.open_gate.send(()).expect("the lookup waits");
    rig.engine.cancel_all();
    rig.wait_ended(second);
    rig.drain();
    assert_eq!(rig.ended_count(first), 1);
    assert!(
        !rig.seen
            .iter()
            .any(|event| matches!(event, OpsEvent::Phase { id, .. } if *id == first)),
        "the cancelled job never started"
    );
    assert_eq!(read(&root.path().join("v0-x/f.txt")), b"old");
}

#[test]
fn a_lookup_that_panics_gives_the_job_an_unknown_volume_and_leaves_the_engine_working() {
    let root = temp();
    put(root.path(), "src/a.txt", b"a");
    let panics = |target: &Path| -> VolumeKey {
        assert!(!target.ends_with("boom"), "the lookup failed");

        volume_by_name(target)
    };
    let mut rig = Rig::hooked(|hooks| hooks.keyer = Some(Arc::new(panics)));
    let src = root.path().join("src");

    let first = rig.start(copy(&src, &["a.txt"], &root.path().join("boom")));
    let second = rig.start(copy(&src, &["a.txt"], &root.path().join("fine")));

    assert_eq!(rig.wait_ended(first).outcome, Outcome::Done);
    assert_eq!(rig.wait_ended(second).outcome, Outcome::Done);
    assert_eq!(read(&root.path().join("boom/a.txt")), b"a");

    // No worker is left counted, and a later job still runs.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while rig.engine.workers() != 0 && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(rig.engine.workers(), 0);
    let third = rig.start(copy(&src, &["a.txt"], &root.path().join("later")));
    assert_eq!(rig.wait_ended(third).outcome, Outcome::Done);
}

#[test]
fn shutdown_cancels_the_jobs_and_returns_as_soon_as_their_workers_are_done() {
    let (root, src) = blocked_scene();
    let mut rig = Rig::new_by_name();
    let jobs: Vec<JobId> = (0..3)
        .map(|index| blocked(&rig, &src, &root.path().join(format!("v{index}-x"))))
        .collect();
    for job in &jobs {
        rig.wait_conflict(*job);
    }

    let started = std::time::Instant::now();
    let finished = rig.engine.shutdown(Duration::from_secs(20));

    assert!(finished);
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    // Every job has ended by the time it returns, and its event has been delivered.
    assert_eq!(rig.engine.workers(), 0);
    rig.drain();
    for job in &jobs {
        assert_eq!(rig.ended_count(*job), 1);
    }
    assert!(
        rig.engine.shutdown(Duration::ZERO),
        "with nothing running it returns at once"
    );
}

#[test]
fn shutdown_gives_up_on_a_worker_that_does_not_answer() {
    let (root, src) = blocked_scene();
    let (mut rig, hang) = hung_first_lookup();
    let first = blocked(&rig, &src, &root.path().join("v0-x"));
    hang.wait_until_stuck();

    let finished = rig.engine.shutdown(Duration::from_millis(200));

    // The job itself is cancelled, but the worker stuck in the lookup cannot be waited for.
    assert!(!finished);
    assert_eq!(rig.wait_ended(first).outcome, Outcome::Cancelled);
    hang.open_gate.send(()).expect("the lookup waits");
    assert!(rig.engine.shutdown(Duration::from_secs(10)));
}

#[test]
fn the_job_limit_gives_busy_and_cancel_all_ends_every_job_once() {
    let (root, src) = blocked_scene();
    let mut rig = Rig::new_by_name();

    let jobs: Vec<JobId> = (0..MAX_JOBS)
        .map(|index| {
            let to = root.path().join(format!("v{index}-x"));
            if index < MAX_WORKERS {
                blocked(&rig, &src, &to)
            } else {
                rig.start(copy(&src, &["f.txt"], &to))
            }
        })
        .collect();

    assert_eq!(
        rig.engine.submit(copy(&src, &["f.txt"], &root.path().join("v-late"))),
        Err(Refused::Busy)
    );

    rig.engine.cancel_all();
    for job in &jobs {
        assert_eq!(rig.wait_ended(*job).outcome, Outcome::Cancelled, "{job:?}");
    }
    rig.drain();

    for job in &jobs {
        assert_eq!(rig.ended_count(*job), 1, "{job:?}");
    }
    assert_eq!(
        rig.engine.snapshot(),
        OpsEvent::Snapshot {
            jobs: vec![],
            questions: vec![]
        }
    );
    assert!(
        rig.engine
            .submit(copy(&src, &["f.txt"], &root.path().join("v-late")))
            .is_ok()
    );
}
