use std::cell::Cell;
use std::path::Path;

use super::*;

#[test]
fn the_question_shows_the_line_and_the_answer_holds_for_the_run_a_no_too() {
    let asked = Cell::new(0);
    let shell = Path::new("/bin/bash");

    let allowing = Shells::default();
    allowing
        .consent(shell, "rm -rf \"old stuff\"", |question| {
            asked.set(asked.get() + 1);
            assert!(
                question.contains(r#""rm -rf \"old stuff\"""#),
                "the line, quoted: {question}"
            );
            assert!(question.contains("/bin/bash"), "{question}");
            true
        })
        .expect("allowed");
    allowing
        .consent(shell, "ls", |_| unreachable!("not asked again"))
        .expect("still allowed");

    let refusing = Shells::default();
    let refused = refusing.consent(shell, "ls", |_| {
        asked.set(asked.get() + 1);
        false
    });
    assert_eq!(refused.expect_err("refused").code(), IpcErrorCode::InvalidInput);
    let again = refusing.consent(shell, "ls", |_| unreachable!("a no is not asked again"));
    assert_eq!(again.expect_err("still refused").code(), IpcErrorCode::InvalidInput);

    assert_eq!(asked.get(), 2);
}

#[test]
fn a_line_too_long_to_show_whole_is_not_put_to_the_user() {
    let shells = Shells::default();

    let refused = shells.consent(Path::new("/bin/sh"), &"x".repeat(3000), |_| unreachable!("not asked"));

    assert_eq!(refused.expect_err("too long").code(), IpcErrorCode::InvalidInput);
}

#[test]
fn a_question_that_panicked_does_not_leave_the_next_one_busy() {
    let shells = Shells::default();
    let asked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = shells.consent(Path::new("/bin/sh"), "ls", |_| panic!("the dialog failed"));
    }));
    assert!(asked.is_err());

    shells
        .consent(Path::new("/bin/sh"), "ls", |_| true)
        .expect("asked again and allowed");
}

#[test]
fn a_second_question_at_once_is_busy() {
    let shells = Shells::default();

    shells
        .consent(Path::new("/bin/sh"), "ls", |_| {
            let busy = shells.consent(Path::new("/bin/sh"), "pwd", |_| unreachable!("not put"));
            assert_eq!(busy.expect_err("busy").code(), IpcErrorCode::Busy);
            true
        })
        .expect("the first is answered");
}

#[test]
fn a_terminal_program_is_asked_about_once_by_its_path_and_never_started_when_refused() {
    let shells = Shells::default();
    let folder = tempfile::tempdir().expect("a folder");
    let asked = Cell::new(0);

    for _ in 0..2 {
        let place = shells.terminals.acquire(TERMINALS).expect("a place");
        let refused = open_terminal(&shells, folder.path(), place, |question| {
            asked.set(asked.get() + 1);
            assert!(question.starts_with("Open terminals with this program?"), "{question}");
            false
        });
        // No terminal on the machine running the tests is a refusal of its own, before any question.
        if let Err(error) = refused {
            assert!(matches!(
                error.code(),
                IpcErrorCode::InvalidInput | IpcErrorCode::Process
            ));
        }
    }

    assert!(asked.get() <= 1, "a no holds: asked {} times", asked.get());
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
