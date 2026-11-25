//! The rules of an active code: expiry, single use, three failures and one attempt at a time.

use std::thread::sleep;
use std::time::Duration;

use pretty_assertions::assert_eq;

use super::{CODE_TTL, MAX_FAILURES, PairingHost, PairingState};

fn host() -> std::sync::Arc<PairingHost> {
    PairingHost::new(CODE_TTL)
}

#[test]
fn a_new_code_is_shown_for_two_minutes() {
    let host = host();
    assert_eq!(host.state(), PairingState::Idle);

    let offer = host.begin().unwrap();

    assert_eq!(offer.code.len(), 9);
    assert_eq!(offer.code.as_bytes()[4], b'-');
    assert_eq!(offer.expires_in, Duration::from_secs(120));
    let PairingState::Waiting {
        expires_in,
        attempts_left,
    } = host.state()
    else {
        panic!("a code is waiting");
    };
    assert!(expires_in > Duration::from_secs(118) && expires_in <= Duration::from_secs(120));
    assert_eq!(attempts_left, MAX_FAILURES);
}

#[test]
fn without_a_code_there_is_no_attempt() {
    assert!(host().start_attempt().is_none());
}

#[test]
fn the_first_success_uses_the_code_up() {
    let host = host();
    host.begin().unwrap();

    let attempt = host.start_attempt().unwrap();
    assert!(attempt.succeed());

    assert_eq!(host.state(), PairingState::Idle);
    assert!(host.start_attempt().is_none());
}

#[test]
fn only_one_attempt_runs_at_a_time() {
    let host = host();
    host.begin().unwrap();

    let first = host.start_attempt().unwrap();
    assert!(host.start_attempt().is_none());

    first.fail();
    assert!(host.start_attempt().is_some());
}

#[test]
fn three_failures_cancel_the_code() {
    let host = host();
    host.begin().unwrap();

    for expected_left in [2, 1] {
        host.start_attempt().unwrap().fail();
        let PairingState::Waiting { attempts_left, .. } = host.state() else {
            panic!("the code is still active");
        };
        assert_eq!(attempts_left, expected_left);
    }
    host.start_attempt().unwrap().fail();

    assert_eq!(host.state(), PairingState::Idle);
    assert!(host.start_attempt().is_none());
}

#[test]
fn an_attempt_that_is_dropped_counts_as_a_failure() {
    let host = host();
    host.begin().unwrap();

    drop(host.start_attempt().unwrap());

    let PairingState::Waiting { attempts_left, .. } = host.state() else {
        panic!("the code is still active");
    };
    assert_eq!(attempts_left, MAX_FAILURES - 1);
    assert!(host.start_attempt().is_some());
}

#[test]
fn an_expired_code_is_refused() {
    let host = PairingHost::new(Duration::from_millis(250));
    host.begin().unwrap();
    let running = host.start_attempt().unwrap();

    sleep(Duration::from_millis(500));

    assert_eq!(host.state(), PairingState::Idle);
    assert!(!running.succeed(), "an attempt that outlives the code does not pair");
    assert!(host.start_attempt().is_none());
}

#[test]
fn cancelling_ends_the_code_and_an_attempt_that_runs() {
    let host = host();
    host.begin().unwrap();
    let running = host.start_attempt().unwrap();

    host.cancel();

    assert_eq!(host.state(), PairingState::Idle);
    assert!(!running.succeed());
}

#[test]
fn a_new_code_replaces_the_old_one_and_starts_with_all_its_tries() {
    let host = host();
    let old = host.begin().unwrap();
    host.start_attempt().unwrap().fail();
    let running = host.start_attempt().unwrap();

    let new = host.begin().unwrap();

    assert!(old.code != new.code);
    assert!(!running.succeed(), "the old code is gone");
    let PairingState::Waiting { attempts_left, .. } = host.state() else {
        panic!("the new code is active");
    };
    assert_eq!(attempts_left, MAX_FAILURES);
    assert!(host.start_attempt().is_some());
}

#[test]
fn a_failure_of_an_old_code_does_not_count_against_the_new_one() {
    let host = host();
    host.begin().unwrap();
    let running = host.start_attempt().unwrap();
    host.begin().unwrap();

    running.fail();

    let PairingState::Waiting { attempts_left, .. } = host.state() else {
        panic!("the new code is active");
    };
    assert_eq!(attempts_left, MAX_FAILURES);
}

#[test]
fn an_attempt_that_sent_nothing_is_released_without_a_failure() {
    let host = host();
    host.begin().unwrap();

    host.start_attempt().unwrap().release();

    let PairingState::Waiting { attempts_left, .. } = host.state() else {
        panic!("the code is still active");
    };
    assert_eq!(attempts_left, MAX_FAILURES);
    assert!(host.start_attempt().is_some(), "the next attempt can start");
}

#[test]
fn the_offer_does_not_print_the_code() {
    let offer = host().begin().unwrap();

    let shown = format!("{offer:?}");

    assert!(
        !shown.contains(&offer.code) && !shown.contains(&offer.code[..4]),
        "{shown}"
    );
}

#[test]
fn the_observer_hears_when_a_code_begins_and_why_it_ends() {
    use std::sync::{Arc, Mutex};

    use super::{Change, PairingEndReason};

    let host = host();
    let heard = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&heard);
    host.observe(move |change| sink.lock().unwrap().push(change));

    host.begin().unwrap();
    host.start_attempt().unwrap().succeed();
    host.begin().unwrap();
    host.begin().unwrap();
    host.cancel();
    host.begin().unwrap();
    for _ in 0..MAX_FAILURES {
        host.start_attempt().unwrap().fail();
    }
    host.cancel();

    use Change::{Began, Ended};
    assert_eq!(
        *heard.lock().unwrap(),
        vec![
            Began,
            Ended(PairingEndReason::Used),
            Began,
            Ended(PairingEndReason::Cancelled),
            Began,
            Ended(PairingEndReason::Cancelled),
            Began,
            Ended(PairingEndReason::TooManyAttempts),
        ]
    );
}
