//! A secret is used for what it was asked for: a passphrase opens a key file here and is never sent to the server, a
//! password is sent to the server and never opens a key file.

use std::time::Duration;

use pairpane_core::model::{ConnectOutcome, ConnectTarget, SecretKind, SecretReason};
use pretty_assertions::assert_eq;

use super::{Harness, count, key_only, needed, needed_because};
use crate::test_server::{Settings, generate_key, write_key};

const PASSPHRASE: &str = "open sesame";
const PASSWORD: &str = "correct horse";

/// A server that takes a password and one key (not the user's own), and a user whose default key is encrypted with
/// `PASSPHRASE`: the passphrase of the local key is also what the server wants as a password.
fn server_with_the_passphrase_as_password() -> Harness {
    let h = Harness::new(Settings {
        password: Some(PASSPHRASE.to_owned()),
        authorized_key: Some(generate_key().public_key().clone()),
        ..Settings::default()
    });

    h.trust();
    write_key(&generate_key(), Some(PASSPHRASE), &h.ssh("id_ed25519"));

    h
}

#[test]
fn a_passphrase_is_never_sent_to_the_server() {
    let h = server_with_the_passphrase_as_password();

    // The default key is locked and the server takes keys: its passphrase is asked for first, as OpenSSH does.
    assert_eq!(
        h.connect(None).expect("answered"),
        needed(SecretKind::Passphrase, false, Some(SecretKind::Password))
    );

    // The passphrase opens the key; the server does not accept that key. It is asked for a password, and the
    // passphrase did not go with that sign-in.
    assert_eq!(
        h.answer(Some((SecretKind::Passphrase, PASSPHRASE))).expect("answered"),
        needed_because(SecretKind::Password, false, None, SecretReason::KeyNotAccepted)
    );
    assert_eq!(h.server.counts.offered(), Vec::<String>::new());

    // The same text typed as the password is the user's own choice.
    assert!(matches!(
        h.answer(Some((SecretKind::Password, PASSPHRASE))).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
}

#[test]
fn a_password_never_opens_a_key() {
    let key = generate_key();
    let h = Harness::new(Settings {
        password: Some("something else".to_owned()),
        authorized_key: Some(key.public_key().clone()),
        ..Settings::default()
    });

    h.trust();
    write_key(&key, Some(PASSPHRASE), &h.ssh("id_ed25519"));

    // The text is the key's passphrase; given as a password it is sent as one, and the key stays shut.
    assert_eq!(
        h.answer(Some((SecretKind::Password, PASSPHRASE))).expect("answered"),
        needed(SecretKind::Password, true, Some(SecretKind::Passphrase))
    );
    assert_eq!(h.server.counts.offered(), [PASSPHRASE]);
}

#[test]
fn a_password_only_server_asks_for_the_password_whatever_keys_there_are() {
    let h = Harness::new(Settings::default());

    h.trust();
    write_key(&generate_key(), Some(PASSPHRASE), &h.ssh("id_ed25519"));

    assert_eq!(
        h.connect(None).expect("answered"),
        needed(SecretKind::Password, false, None)
    );
}

#[test]
fn a_passphrase_that_opened_nothing_is_no_reason_to_say_a_key_was_not_accepted() {
    // No key is looked at on a server that does not take keys, so nothing was opened with the passphrase.
    let h = Harness::new(Settings::default());

    h.trust();
    write_key(&generate_key(), Some(PASSPHRASE), &h.ssh("id_ed25519"));

    assert_eq!(
        h.answer(Some((SecretKind::Passphrase, PASSPHRASE))).expect("answered"),
        needed(SecretKind::Password, false, None)
    );
}

/// A server that takes a password and a key, and a target whose named key file holds that key, encrypted.
fn named_locked_key_and_a_password() -> (Harness, ConnectTarget) {
    let key = generate_key();
    let h = Harness::new(Settings {
        authorized_key: Some(key.public_key().clone()),
        ..Settings::default()
    });
    let mut target = h.target();

    h.trust();
    write_key(&key, Some(PASSPHRASE), &h.home.path().join("named_key"));
    target.key_file = Some("~/named_key".to_owned());

    (h, target)
}

#[test]
fn a_named_key_asks_for_its_passphrase_even_when_the_server_takes_a_password() {
    let (h, target) = named_locked_key_and_a_password();

    let ask = |answer| h.connect_with(&target, answer).expect("answered");
    let given = |text: &str| Some(Harness::secret(SecretKind::Passphrase, text));

    assert_eq!(
        ask(None),
        needed(SecretKind::Passphrase, false, Some(SecretKind::Password))
    );
    assert_eq!(
        ask(given("nope")),
        needed(SecretKind::Passphrase, true, Some(SecretKind::Password))
    );
    assert!(matches!(ask(given(PASSPHRASE)), ConnectOutcome::Connected { .. }));
    assert_eq!(h.server.counts.offered(), Vec::<String>::new());
}

#[test]
fn after_a_named_key_was_opened_and_not_accepted_the_password_is_asked_for_and_ends_it() {
    let h = Harness::new(Settings {
        authorized_key: Some(generate_key().public_key().clone()),
        ..Settings::default()
    });
    let mut target = h.target();

    h.trust();
    write_key(&generate_key(), Some(PASSPHRASE), &h.home.path().join("named_key"));
    target.key_file = Some("~/named_key".to_owned());

    let with = |kind, text: &str| h.connect_with(&target, Some(Harness::secret(kind, text)));

    assert_eq!(
        h.connect_with(&target, None).expect("answered"),
        needed(SecretKind::Passphrase, false, Some(SecretKind::Password))
    );
    assert_eq!(
        with(SecretKind::Passphrase, PASSPHRASE).expect("answered"),
        needed_because(SecretKind::Password, false, None, SecretReason::KeyNotAccepted)
    );
    // The key is locked again for this call, yet a wrong password is a wrong password, not a second question.
    assert_eq!(
        with(SecretKind::Password, "wrong").expect("answered"),
        needed(SecretKind::Password, true, Some(SecretKind::Passphrase))
    );
    assert!(matches!(
        with(SecretKind::Password, PASSWORD).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
}

/// A server that takes a password and the key that the default key file holds, which is encrypted.
fn locked_default_key_and_a_password() -> Harness {
    let key = generate_key();
    let h = Harness::new(Settings {
        authorized_key: Some(key.public_key().clone()),
        ..Settings::default()
    });

    h.trust();
    write_key(&key, Some(PASSPHRASE), &h.ssh("id_ed25519"));

    h
}

#[test]
fn a_named_key_that_is_asked_for_can_be_skipped_by_giving_the_password() {
    let (h, target) = named_locked_key_and_a_password();

    assert_eq!(
        h.connect_to(&target, None).expect("answered"),
        needed(SecretKind::Passphrase, false, Some(SecretKind::Password))
    );

    // The password was offered as the other kind: it goes to the server, and the key is never opened.
    assert!(matches!(
        h.connect_to(&target, Some(PASSWORD)).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
    assert_eq!(h.server.counts.offered(), [PASSWORD]);
}

#[test]
fn a_locked_default_key_asks_for_its_passphrase_first_and_opens_with_it() {
    let h = locked_default_key_and_a_password();

    assert_eq!(
        h.connect(None).expect("answered"),
        needed(SecretKind::Passphrase, false, Some(SecretKind::Password))
    );

    // The key opens and signs in, and no password is sent.
    assert!(matches!(
        h.answer(Some((SecretKind::Passphrase, PASSPHRASE))).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
    assert_eq!(h.server.counts.offered(), Vec::<String>::new());
}

#[test]
fn a_locked_default_key_can_be_skipped_by_giving_the_password() {
    let h = locked_default_key_and_a_password();

    assert_eq!(
        h.connect(None).expect("answered"),
        needed(SecretKind::Passphrase, false, Some(SecretKind::Password))
    );
    assert!(matches!(
        h.answer(Some((SecretKind::Password, PASSWORD))).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
    assert_eq!(h.server.counts.offered(), [PASSWORD]);
}

#[test]
fn a_wrong_answer_of_either_kind_keeps_the_other_one_available() {
    let h = locked_default_key_and_a_password();

    assert_eq!(
        h.connect(None).expect("answered"),
        needed(SecretKind::Passphrase, false, Some(SecretKind::Password))
    );
    assert_eq!(
        h.answer(Some((SecretKind::Passphrase, "nope"))).expect("answered"),
        needed(SecretKind::Passphrase, true, Some(SecretKind::Password))
    );
    assert_eq!(
        h.answer(Some((SecretKind::Password, "not it"))).expect("answered"),
        needed(SecretKind::Password, true, Some(SecretKind::Passphrase))
    );
    assert!(matches!(
        h.answer(Some((SecretKind::Passphrase, PASSPHRASE))).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
}

#[test]
fn a_second_step_of_the_sign_in_is_never_answered_with_the_password() {
    let h = Harness::new(Settings {
        keyboard_interactive: true,
        two_step: true,
        ..Settings::default()
    });

    h.trust();

    assert_eq!(
        h.connect(None).expect("answered"),
        needed(SecretKind::Password, false, None)
    );

    // The password is the answer to the first prompt only; the code that the second one asks for is not it.
    assert_eq!(
        h.answer(Some((SecretKind::Password, PASSWORD))).expect("answered"),
        needed(SecretKind::Password, true, None)
    );
    assert!(h.server.counts.verification.lock().expect("answers").is_empty());
    assert_eq!(
        h.server.counts.offered(),
        [PASSWORD, PASSWORD],
        "as a password, and to the first prompt"
    );
}

#[test]
fn a_site_opened_with_a_passphrase_opens_again_with_it_and_sends_nothing() {
    let key = generate_key();
    let h = Harness::new(key_only(&key));

    h.trust();
    write_key(&key, Some(PASSPHRASE), &h.ssh("id_ed25519"));
    h.server.put("kept/file", b"");

    let ConnectOutcome::Connected { site, .. } =
        h.answer(Some((SecretKind::Passphrase, PASSPHRASE))).expect("answered")
    else {
        panic!("not connected");
    };
    let fs = h.pool.fs(site).expect("a site");

    h.server.drop_connections();
    std::thread::sleep(Duration::from_millis(200));

    assert_eq!(count(&*fs, "/kept"), 1);
    assert_eq!(h.server.counts.offered(), Vec::<String>::new());
}
