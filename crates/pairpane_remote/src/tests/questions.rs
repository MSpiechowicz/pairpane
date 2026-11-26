//! An answer is only taken for the question that was asked, and the question outlives an attempt that failed for
//! another reason.

use std::io::ErrorKind;
use std::net::TcpListener;
use std::time::Duration;

use pairpane_core::model::{ConnectOutcome, SecretKind};
use pretty_assertions::assert_eq;

use super::{Harness, needed, quick, target_of};
use crate::test_server::{Settings, generate_key, write_key};
use crate::{AgentSource, Timeouts};

const PASSWORD: &str = "correct horse";

#[test]
fn an_answer_is_only_taken_for_the_question_that_was_asked() {
    let h = Harness::new(Settings::default());
    let give = |kind, text: &str| h.connect_with(&h.target(), Some(Harness::secret(kind, text)));

    h.trust();

    // No question yet.
    let error = give(SecretKind::Password, PASSWORD).expect_err("refused");

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "the answer does not match the question");
    assert_eq!(
        h.server.counts.connections.load(std::sync::atomic::Ordering::Relaxed),
        0
    );

    // The question is for the password, and a passphrase (which is not on offer) is not an answer to it.
    assert_eq!(
        h.connect(None).expect("answered"),
        needed(SecretKind::Password, false, None)
    );

    let error = give(SecretKind::Passphrase, PASSWORD).expect_err("refused");

    assert_eq!(error.to_string(), "the answer does not match the question");
    assert_eq!(
        h.server.counts.connections.load(std::sync::atomic::Ordering::Relaxed),
        1
    );

    // The question stands after a refused answer.
    assert!(matches!(
        give(SecretKind::Password, PASSWORD).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
}

#[test]
fn a_question_outlives_an_attempt_that_failed_for_another_reason() {
    let h = Harness::new(Settings::default());

    h.trust();

    assert_eq!(
        h.connect(None).expect("answered"),
        needed(SecretKind::Password, false, None)
    );

    // The server is down for the attempt with the answer, and the connection is refused.
    h.server.refuse_new_connections();
    assert!(h.answer(Some((SecretKind::Password, PASSWORD))).is_err());

    // Up again, the same answer is taken: it is not "does not match the question".
    h.server.accept_new_connections();

    assert!(matches!(
        h.connect_with(&h.target(), Some(Harness::secret(SecretKind::Password, PASSWORD)))
            .expect("connected"),
        ConnectOutcome::Connected { .. }
    ));
}

#[test]
fn a_question_outlives_a_timeout() {
    let h = Harness::with(
        Settings::default(),
        Timeouts {
            handshake: Duration::from_millis(300),
            ..quick()
        },
        AgentSource::None,
    );
    // Takes the connection and never speaks.
    let silent = TcpListener::bind(("127.0.0.1", 0)).expect("a port");
    let target = target_of(silent.local_addr().expect("an address"));

    h.pool.assume_asked(&target, SecretKind::Password);

    for _ in 0..2 {
        let error = h.connect_to(&target, Some(PASSWORD)).expect_err("timed out");

        assert_eq!(error.kind(), ErrorKind::TimedOut);
    }
}

#[test]
fn a_rejected_answer_replaces_the_question_with_the_new_one() {
    let key = generate_key();
    let h = Harness::new(Settings {
        authorized_key: Some(key.public_key().clone()),
        ..Settings::default()
    });

    h.trust();
    write_key(&key, Some("open sesame"), &h.ssh("id_ed25519"));

    assert_eq!(
        h.connect(None).expect("answered"),
        needed(SecretKind::Passphrase, false, Some(SecretKind::Password))
    );
    // A wrong password: the question is now for the password, with the passphrase offered instead.
    assert_eq!(
        h.answer(Some((SecretKind::Password, "nope"))).expect("answered"),
        needed(SecretKind::Password, true, Some(SecretKind::Passphrase))
    );
    assert!(matches!(
        h.answer(Some((SecretKind::Passphrase, "open sesame")))
            .expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
}

#[test]
fn a_connect_that_finds_the_site_open_forgets_the_question_that_was_left() {
    let h = Harness::new(Settings::default());

    h.trust();

    let site = h.site(Some(PASSWORD));

    // A question for this server that is still remembered (a second dialog that was open meanwhile).
    h.pool.assume_asked(&h.target(), SecretKind::Password);

    assert!(matches!(
        h.connect(None).expect("the site that is open"),
        ConnectOutcome::Connected { .. }
    ));

    // After the site is closed the old question is gone: a password with no question is not taken.
    h.pool.disconnect(site);

    let error = h
        .connect_with(&h.target(), Some(Harness::secret(SecretKind::Password, PASSWORD)))
        .expect_err("no question");

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "the answer does not match the question");
}
