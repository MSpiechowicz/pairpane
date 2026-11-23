use std::cell::Cell;
use std::path::Path;

use super::*;

#[test]
fn the_user_is_asked_once_a_yes_holds_and_a_no_asks_again_next_time() {
    let shells = Shells::default();
    let asked = Cell::new(0);
    let shell = Path::new("/bin/bash");

    let refused = shells.consent(shell, |question| {
        asked.set(asked.get() + 1);
        assert!(question.contains("/bin/bash"), "{question}");
        false
    });
    assert_eq!(refused.expect_err("refused").code(), IpcErrorCode::InvalidInput);

    shells
        .consent(shell, |_| {
            asked.set(asked.get() + 1);
            true
        })
        .expect("allowed");
    shells
        .consent(shell, |_| unreachable!("not asked again"))
        .expect("still allowed");

    assert_eq!(asked.get(), 2);
}

#[test]
fn a_question_that_panicked_does_not_leave_the_next_one_busy() {
    let shells = Shells::default();
    let asked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = shells.consent(Path::new("/bin/sh"), |_| panic!("the dialog failed"));
    }));
    assert!(asked.is_err());

    shells
        .consent(Path::new("/bin/sh"), |_| true)
        .expect("asked again and allowed");
}

#[test]
fn a_second_question_at_once_is_busy() {
    let shells = Shells::default();
    let _asking = shells.asking.lock().expect("the lock");

    let busy = shells.consent(Path::new("/bin/sh"), |_| unreachable!("not put"));

    assert_eq!(busy.expect_err("busy").code(), IpcErrorCode::Busy);
}

#[test]
fn at_most_four_commands_run_and_a_stop_of_one_that_ended_is_not_found() {
    let shells = Shells::default();
    let runs: Vec<_> = (0..RUNNING).map(|_| shells.register().expect("a place")).collect();

    assert_eq!(shells.register().expect_err("full").code(), IpcErrorCode::Busy);

    let (id, stop, _) = &runs[0];
    shells.stop(*id).expect("stopped");
    assert!(stop.asked());

    shells.forget(*id);
    assert_eq!(shells.stop(*id).expect_err("ended").code(), IpcErrorCode::NotFound);
}

#[test]
fn the_exit_raises_every_stop_flag() {
    let shells = Shells::default();
    let (_, first, _permit) = shells.register().expect("a place");
    let (_, second, _other) = shells.register().expect("a place");

    shells.shut_down();

    assert!(first.asked() && second.asked());
}

#[test]
fn a_failure_to_start_tells_no_path() {
    let error = start_error(&io::Error::new(io::ErrorKind::NotFound, "/home/me/secret: not found"));

    assert_eq!(error.code, IpcErrorCode::Process);
    assert!(!error.message.contains("/home"));
}
