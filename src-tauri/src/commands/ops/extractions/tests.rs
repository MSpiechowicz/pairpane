//! The password of an extraction job that ends because it was wrong is forgotten, and only that one.

use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use pairpane_core::archive::{self, PasswordFault};
use pairpane_core::model::{IpcError, IpcErrorCode};

use crate::error::CommandError;

use super::*;

const ARCHIVE: &str = "/data/pack.zip";

fn ended(id: u64, error: IpcError) -> OpsEvent {
    OpsEvent::Ended {
        id: JobId(id),
        outcome: Outcome::Failed { error },
        skipped: 0,
        errors: Vec::new(),
        more_errors: 0,
    }
}

fn wrong() -> IpcError {
    IpcError::new(
        IpcErrorCode::PasswordRequired,
        archive::fault_text(PasswordFault::Wrong),
    )
}

/// An archive with a password remembered for it, and that password.
fn remembered(word: &str) -> (Archives, Arc<Password>) {
    let archives = Archives::default();
    archives.remember(PathBuf::from(ARCHIVE), Password::new(word.to_owned()));
    let used = archives.password(Path::new(ARCHIVE)).expect("remembered");

    (archives, used)
}

fn track(extractions: &Extractions, archives: &Archives, used: &Arc<Password>, id: u64) {
    extractions
        .submit(archives, PathBuf::from(ARCHIVE), Some(Arc::clone(used)), || {
            Ok(JobId(id))
        })
        .expect("queued");
}

#[test]
fn the_password_a_job_ends_with_as_wrong_is_forgotten() {
    let (archives, used) = remembered("one");
    let extractions = Extractions::default();
    track(&extractions, &archives, &used, 1);

    extractions.observe(&ended(1, wrong()));

    assert!(archives.password(Path::new(ARCHIVE)).is_none());
}

#[test]
fn another_failure_a_missing_password_and_a_job_nobody_tracks_forget_nothing() {
    let (archives, used) = remembered("one");
    let extractions = Extractions::default();
    track(&extractions, &archives, &used, 1);
    track(&extractions, &archives, &used, 2);

    extractions.observe(&ended(1, IpcError::new(IpcErrorCode::Io, "the disk is full")));
    extractions.observe(&ended(
        2,
        IpcError::new(
            IpcErrorCode::PasswordRequired,
            archive::fault_text(PasswordFault::Needed),
        ),
    ));
    extractions.observe(&ended(3, wrong()));

    assert!(archives.password(Path::new(ARCHIVE)).is_some());

    // Its entry went with its end: a second end of the same job does nothing either.
    extractions.observe(&ended(1, wrong()));
    assert!(archives.password(Path::new(ARCHIVE)).is_some());
}

#[test]
fn a_password_the_user_gave_after_the_job_started_is_kept() {
    let (archives, used) = remembered("old");
    let extractions = Extractions::default();
    track(&extractions, &archives, &used, 1);
    archives.remember(PathBuf::from(ARCHIVE), Password::new("new".to_owned()));

    extractions.observe(&ended(1, wrong()));

    assert!(archives.password(Path::new(ARCHIVE)).is_some());
}

#[test]
fn a_job_that_ends_before_it_is_registered_waits_for_its_entry() {
    let (archives, used) = remembered("one");
    let extractions = Extractions::default();

    thread::scope(|scope| {
        let ended_at_once = extractions.submit(&archives, PathBuf::from(ARCHIVE), Some(Arc::clone(&used)), || {
            // The engine's worker is quick: the job fails while the command is still registering it.
            scope.spawn(|| extractions.observe(&ended(1, wrong())));
            thread::sleep(Duration::from_millis(100));

            Ok(JobId(1))
        });

        assert_eq!(ended_at_once.expect("queued"), JobId(1));
    });

    assert!(archives.password(Path::new(ARCHIVE)).is_none());
}

#[test]
fn an_event_made_on_the_submitting_thread_does_not_wait_for_the_lock_it_holds_and_ends_its_job() {
    let (archives, used) = remembered("one");
    let extractions = Extractions::default();

    let id = extractions
        .submit(&archives, PathBuf::from(ARCHIVE), Some(Arc::clone(&used)), || {
            // A job that could not be started ends on the thread that queued it.
            extractions.observe(&ended(
                1,
                IpcError::new(IpcErrorCode::Io, "could not start the transfer"),
            ));

            Ok(JobId(1))
        })
        .expect("queued");

    assert_eq!(id, JobId(1));
    assert!(extractions.lock().is_empty(), "its entry went with its end");
}

#[test]
fn another_job_that_ends_on_the_submitting_thread_is_settled_when_the_submission_is_done() {
    let (archives, used) = remembered("one");
    let extractions = Extractions::default();
    track(&extractions, &archives, &used, 1);
    track(&extractions, &archives, &used, 2);

    extractions
        .submit(&archives, PathBuf::from(ARCHIVE), None, || {
            // Jobs that were waiting are stranded by the new one: one failed for a wrong password, one for another reason.
            extractions.observe(&ended(1, IpcError::new(IpcErrorCode::Io, "the disk is full")));
            extractions.observe(&ended(2, wrong()));

            Ok(JobId(3))
        })
        .expect("queued");

    assert!(extractions.lock().is_empty(), "both entries went");
    assert!(
        archives.password(Path::new(ARCHIVE)).is_none(),
        "the wrong password was forgotten"
    );
}

#[test]
fn a_submission_that_fails_still_settles_the_jobs_that_ended_meanwhile() {
    let (archives, used) = remembered("one");
    let extractions = Extractions::default();
    track(&extractions, &archives, &used, 1);

    let queued = extractions.submit(&archives, PathBuf::from(ARCHIVE), None, || {
        extractions.observe(&ended(1, wrong()));

        Err(CommandError::new(IpcErrorCode::Io, "could not queue"))
    });

    assert!(queued.is_err());
    assert!(extractions.lock().is_empty());
    assert!(archives.password(Path::new(ARCHIVE)).is_none());
}

#[test]
fn a_submission_that_panics_leaves_the_thread_settling_its_ends_at_once() {
    let (archives, used) = remembered("one");
    let extractions = Extractions::default();
    track(&extractions, &archives, &used, 1);

    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = extractions.submit(&archives, PathBuf::from(ARCHIVE), None, || -> CommandResult<JobId> {
            panic!("the queue broke")
        });
    }));
    assert!(panicked.is_err());

    // The lock is free again and this thread no longer sets its ends aside.
    extractions.observe(&ended(1, wrong()));

    assert!(extractions.lock().is_empty());
    assert!(archives.password(Path::new(ARCHIVE)).is_none());
}
