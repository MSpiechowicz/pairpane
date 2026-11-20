//! Pause, cancel, answers and snapshots of a running job, and a job that panics.

use std::path::Path;

use pretty_assertions::assert_eq;

use crate::model::{Answer, ControlAction, IpcErrorCode, JobId, JobPhase, OpsEvent, Outcome};

use super::hooks::Hooks;
use super::testing::{Rig, blocked, blocked_scene, copy, put, read, temp, view_of};
use super::{AnswerError, UnknownJob};

#[test]
fn a_queued_job_cancels_at_once_without_waiting_for_its_volume() {
    let (root, src) = blocked_scene();
    let mut rig = Rig::new_by_name();

    let first = blocked(&rig, &src, &root.path().join("same-a"));
    rig.wait_conflict(first);
    let second = rig.start(copy(&src, &["f.txt"], &root.path().join("same-b")));

    rig.engine.control(second, ControlAction::Cancel).expect("known job");

    assert_eq!(rig.wait_ended(second).outcome, Outcome::Cancelled);
    assert_eq!(view_of(&rig.engine, first).phase, JobPhase::Waiting);
    assert!(!root.path().join("same-b").exists());
    assert_eq!(rig.engine.control(second, ControlAction::Cancel), Err(UnknownJob));
}

#[test]
fn cancel_wakes_a_job_waiting_for_an_answer() {
    let (root, src) = blocked_scene();
    let mut rig = Rig::new();

    let job = blocked(&rig, &src, &root.path().join("dst"));
    rig.wait_conflict(job);

    rig.engine.control(job, ControlAction::Cancel).expect("known job");

    assert_eq!(rig.wait_ended(job).outcome, Outcome::Cancelled);
    assert_eq!(read(&root.path().join("dst/f.txt")), b"old");
    // The listener hears that the job is stopping before it hears that it has stopped.
    let stopping = rig.seen.iter().position(|event| {
        matches!(
            event,
            OpsEvent::Phase {
                phase: JobPhase::Cancelling,
                ..
            }
        )
    });
    let ended = rig
        .seen
        .iter()
        .position(|event| matches!(event, OpsEvent::Ended { .. }));
    assert!(stopping < ended, "{:#?}", rig.seen);
    assert!(stopping.is_some());
}

#[test]
fn unknown_jobs_and_stale_questions_are_refused() {
    let root = temp();
    for name in ["a", "b"] {
        put(root.path(), &format!("src/{name}"), b"new");
        put(root.path(), &format!("dst/{name}"), b"old");
    }
    let mut rig = Rig::new();

    assert_eq!(rig.engine.control(JobId(99), ControlAction::Pause), Err(UnknownJob));
    assert_eq!(
        rig.engine.answer(JobId(99), 1, Answer::Skip, false),
        Err(AnswerError::UnknownJob)
    );

    let job = rig.copy_in(root.path(), &["a", "b"]);
    let first = rig.answer_next(job, Answer::Skip, false);
    let second = rig.wait_conflict(job);

    // The job waits on its second question: the first one is answered already, and a made-up one never existed.
    for stale in [first.question, second.question + 1] {
        assert_eq!(
            rig.engine.answer(job, stale, Answer::Overwrite, false),
            Err(AnswerError::StaleQuestion)
        );
    }
    rig.engine
        .answer(job, second.question, Answer::Skip, false)
        .expect("open question");
    rig.wait_ended(job);

    assert_eq!(read(&root.path().join("dst/a")), b"old");
    assert_eq!(
        rig.engine.answer(job, second.question, Answer::Skip, false),
        Err(AnswerError::UnknownJob)
    );
}

#[test]
fn a_snapshot_lists_unfinished_jobs_with_their_open_questions() {
    let (root, src) = blocked_scene();
    let mut rig = Rig::new();

    let job = blocked(&rig, &src, &root.path().join("dst"));
    let question = rig.wait_conflict(job);

    let OpsEvent::Snapshot { jobs, questions } = rig.engine.snapshot() else {
        unreachable!("a snapshot");
    };

    assert_eq!(jobs.len(), 1);
    assert_eq!(
        (jobs[0].id, jobs[0].phase, jobs[0].label.as_str()),
        (job, JobPhase::Waiting, "f.txt")
    );
    assert_eq!(questions, [question]);
    rig.engine.cancel_all();
    rig.wait_ended(job);
}

#[test]
fn pause_holds_the_bytes_and_resume_lets_them_go() {
    let root = temp();
    put(root.path(), "src/a.txt", &vec![1; 3 * 1024 * 1024]);
    let mut rig = Rig::paused_at_start();

    let job = rig.copy_in(root.path(), &["a.txt"]);
    rig.wait(|event| matches!(event, OpsEvent::Paused { paused: true, .. }));
    assert_eq!(
        rig.engine.control(job, ControlAction::Pause),
        Ok(()),
        "pausing twice is fine"
    );
    rig.drain();

    let held = view_of(&rig.engine, job);
    assert!(held.paused);
    assert_eq!(held.done_bytes, 0);
    assert_eq!(rig.ended_count(job), 0);
    assert!(!root.path().join("dst/a.txt").exists());
    let pauses = |paused: bool| {
        rig.seen
            .iter()
            .filter(|event| matches!(event, OpsEvent::Paused { paused: p, .. } if *p == paused))
            .count()
    };
    assert_eq!((pauses(true), pauses(false)), (1, 0));

    rig.engine.control(job, ControlAction::Resume).expect("known job");
    assert_eq!(rig.wait_ended(job).outcome, Outcome::Done);
    assert_eq!(read(&root.path().join("dst/a.txt")).len(), 3 * 1024 * 1024);
    assert!(
        rig.seen
            .iter()
            .any(|event| matches!(event, OpsEvent::Paused { paused: false, .. }))
    );
}

#[test]
fn cancel_wakes_a_paused_job() {
    let root = temp();
    put(root.path(), "src/a.txt", b"x");
    let mut rig = Rig::paused_at_start();

    let job = rig.copy_in(root.path(), &["a.txt"]);
    rig.wait(|event| matches!(event, OpsEvent::Paused { paused: true, .. }));
    rig.engine.control(job, ControlAction::Cancel).expect("known job");

    assert_eq!(rig.wait_ended(job).outcome, Outcome::Cancelled);
    assert!(
        !root.path().join("dst").exists(),
        "the folder the job made is removed again"
    );
}

#[test]
fn a_sink_may_call_back_into_the_engine_at_any_event() {
    let root = temp();
    put(root.path(), "src/a.txt", b"x");
    let mut rig = Rig::with(Hooks::default(), |engine, event| {
        engine.snapshot();
        if let OpsEvent::Added { job } = event {
            engine.control(job.id, ControlAction::Resume).expect("known job");
        }
        assert!(engine.submit(copy(Path::new("/"), &[], Path::new("/"))).is_err());
    });

    let ended = rig.run_copy(root.path(), &["a.txt"]);

    assert_eq!(ended.outcome, Outcome::Done);
}

#[test]
fn a_panic_in_a_job_still_ends_it_once_and_leaves_the_engine_usable() {
    let root = temp();
    put(root.path(), "src/a.txt", b"x");
    let mut rig = Rig::hooked(|hooks| hooks.panic_on_start = true);

    let job = rig.copy_in(root.path(), &["a.txt"]);
    let ended = rig.wait_ended(job);
    rig.drain();

    let Outcome::Failed { error } = ended.outcome else {
        panic!("{:?}", ended.outcome);
    };
    assert_eq!(error.code, IpcErrorCode::Internal);
    assert_eq!(rig.ended_count(job), 1);
    assert_eq!(
        rig.engine.snapshot(),
        OpsEvent::Snapshot {
            jobs: vec![],
            questions: vec![]
        }
    );
}

#[test]
fn the_last_paused_event_is_the_state_the_job_is_in_however_the_calls_interleave() {
    let (root, src) = blocked_scene();
    let mut rig = Rig::new();
    let job = blocked(&rig, &src, &root.path().join("dst"));
    rig.wait_conflict(job);

    // Several threads pause and resume the one job at the same time.
    std::thread::scope(|scope| {
        for thread in 0..6 {
            let engine = rig.engine.clone();

            scope.spawn(move || {
                for round in 0..1_500 {
                    let action = if (round + thread) % 2 == 0 {
                        ControlAction::Pause
                    } else {
                        ControlAction::Resume
                    };

                    engine.control(job, action).expect("known job");
                }
            });
        }
    });
    rig.drain();

    let told: Vec<bool> = rig
        .seen
        .iter()
        .filter_map(|event| match event {
            OpsEvent::Paused { paused, .. } => Some(*paused),
            _ => None,
        })
        .collect();
    let truth = rig.engine.is_paused(job);

    assert_eq!(told.last(), Some(&truth), "{} events", told.len());
    assert_eq!(view_of(&rig.engine, job).paused, truth);
    // Each event is a change: the listener is never told the same thing twice in a row.
    assert!(told.windows(2).all(|pair| pair[0] != pair[1]));
    rig.engine.cancel_all();
}
