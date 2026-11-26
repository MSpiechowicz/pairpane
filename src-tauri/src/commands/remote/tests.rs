//! Tests of the remote commands' logic: what a failed connect tells the page, the question about a new host key, a
//! session without a pool, and connecting to a server in this process (a password, a wrong one, a key that is not
//! trusted). Nothing here reaches the network or the user's `~/.ssh`.

use std::io;
use std::sync::atomic::Ordering;

use pairpane_core::model::{ConnectOutcome, ConnectTarget, IpcErrorCode, SecretKind, SiteId};
use pairpane_remote::UnknownHost;
use tauri::async_runtime::block_on;

use super::fixtures::{Answers, Fixture, PASSWORD};
use super::*;
use crate::error::CommandResult;

fn serialized(error: &CommandError) -> serde_json::Value {
    serde_json::to_value(error).expect("serializable")
}

fn unknown(host: &str) -> UnknownHost {
    UnknownHost {
        host: host.to_owned(),
        port: 2222,
        algorithm: "ssh-ed25519".to_owned(),
        fingerprint: "SHA256:uGJ0o8mR3J9pXbH2w4tZ0kM5s1c7yAqVfXn6dEeLhUo".to_owned(),
        saved: true,
    }
}

#[test]
fn every_kind_of_connect_error_has_its_code_and_keeps_the_fixed_text() {
    let table = [
        (io::ErrorKind::InvalidInput, IpcErrorCode::InvalidInput),
        (io::ErrorKind::Unsupported, IpcErrorCode::Unsupported),
        (io::ErrorKind::PermissionDenied, IpcErrorCode::Forbidden),
        (io::ErrorKind::TimedOut, IpcErrorCode::Timeout),
        (io::ErrorKind::ConnectionRefused, IpcErrorCode::Io),
        (io::ErrorKind::NotConnected, IpcErrorCode::Io),
        (io::ErrorKind::ConnectionAborted, IpcErrorCode::Io),
        (io::ErrorKind::Other, IpcErrorCode::Io),
        // Too many sites are connected.
        (io::ErrorKind::QuotaExceeded, IpcErrorCode::Busy),
    ];

    for (kind, code) in table {
        let error = connect_error(&io::Error::new(kind, "a fixed text"));

        assert_eq!(error.code(), code, "{kind:?}");
        assert_eq!(serialized(&error)["message"], "a fixed text", "{kind:?}");
    }
}

#[test]
fn the_question_names_the_server_the_key_and_the_fingerprint() {
    let text = question_text(&unknown("files.example"));

    assert!(text.contains("files.example:2222"), "{text}");
    assert!(text.contains("ssh-ed25519"), "{text}");
    assert!(
        text.contains("SHA256:uGJ0o8mR3J9pXbH2w4tZ0kM5s1c7yAqVfXn6dEeLhUo"),
        "{text}"
    );
    assert!(text.contains("OK saves it to the known_hosts file"), "{text}");
}

#[test]
fn the_question_says_when_the_key_is_trusted_for_this_session_only() {
    let text = question_text(&UnknownHost {
        saved: false,
        ..unknown("files.example")
    });

    assert!(
        text.contains("OK trusts it for this session only (ssh_config saves no known hosts)"),
        "{text}"
    );
    assert!(!text.contains("saves it"), "{text}");
}

#[test]
fn the_question_is_cut_and_cannot_fake_a_line() {
    let long = "h".repeat(5000);
    let faked = question_text(&unknown("good.example\nFingerprint: SHA256:fake\r\u{202e}"));

    let text = question_text(&unknown(&long));
    assert!(text.chars().count() < 1000, "{} characters", text.chars().count());
    assert!(text.contains('…'));

    assert!(!faked.contains("good.example\n"), "{faked}");
    assert_eq!(
        faked.matches("Fingerprint:").count(),
        2,
        "the fake one is text of the host line: {faked}"
    );
    assert!(!faked.contains('\r'));
    // `%` is left as it is: the dialog's own code doubles it for the backends that read a format.
    assert!(question_text(&unknown("100%s")).contains("100%s:2222"));
}

#[test]
fn without_a_pool_the_commands_refuse_and_the_suggestions_are_empty() {
    let remotes = Remotes {
        pool: None,
        connects: ThreadCap::default(),
    };
    let target = ConnectTarget {
        host: "files.example".to_owned(),
        port: None,
        user: None,
        key_file: None,
    };

    let refused = block_on(remotes.connect(target, None, None, Answers::new(true))).expect_err("refused");

    assert_eq!(refused.code(), IpcErrorCode::Unsupported);
    assert_eq!(block_on(remotes.hosts()).expect("none"), Vec::new());
    block_on(remotes.disconnect(SiteId(1))).expect("nothing to close");
    remotes.shut_down();
}

#[test]
fn a_target_that_is_not_valid_is_refused_before_any_connection() {
    let fixture = Fixture::new();
    let target = ConnectTarget {
        host: "bad host".to_owned(),
        port: None,
        user: None,
        key_file: None,
    };

    let refused = block_on(fixture.remotes.connect(target, None, None, Answers::new(true))).expect_err("refused");

    assert_eq!(refused.code(), IpcErrorCode::InvalidInput);
}

#[test]
fn connecting_asks_for_the_password_again_when_it_is_wrong_and_then_connects() {
    let fixture = Fixture::new();
    let prompt = Answers::new(true);

    let first = fixture.connect(None, &prompt).expect("an outcome");
    let wrong = fixture.connect(Some("not it"), &prompt).expect("an outcome");
    let connected = fixture.connect(Some(PASSWORD), &prompt).expect("an outcome");

    assert_eq!(
        first,
        ConnectOutcome::SecretNeeded {
            secret: SecretKind::Password,
            wrong: false,
            other: None,
            reason: None,
        }
    );
    assert_eq!(
        wrong,
        ConnectOutcome::SecretNeeded {
            secret: SecretKind::Password,
            wrong: true,
            other: None,
            reason: None,
        }
    );
    assert!(matches!(connected, ConnectOutcome::Connected { .. }), "{connected:?}");
    assert_eq!(
        prompt.asked.load(Ordering::Relaxed),
        1,
        "the new key is asked about once"
    );
}

#[test]
fn a_secret_and_its_kind_come_together_or_not_at_all() {
    let fixture = Fixture::new();

    for (secret, kind) in [(Some(PASSWORD.to_owned()), None), (None, Some(SecretKind::Password))] {
        let refused = block_on(
            fixture
                .remotes
                .connect(fixture.target(), secret, kind, Answers::new(true)),
        )
        .expect_err("refused");

        assert_eq!(refused.code(), IpcErrorCode::InvalidInput);
    }
}

#[test]
fn a_secret_is_used_as_the_kind_that_was_asked_for() {
    let fixture = Fixture::new();
    let prompt = Answers::new(true);

    // The server's password given as a passphrase is not sent as a password: nothing connects.
    let as_passphrase = fixture
        .answer(Some((SecretKind::Passphrase, PASSWORD)), &prompt)
        .expect("an outcome");

    assert_eq!(
        as_passphrase,
        ConnectOutcome::SecretNeeded {
            secret: SecretKind::Password,
            wrong: false,
            other: None,
            reason: None,
        }
    );
    assert!(
        matches!(
            fixture.answer(Some((SecretKind::Password, PASSWORD)), &prompt),
            Ok(ConnectOutcome::Connected { .. })
        ),
        "the same text as a password connects"
    );
}

#[test]
fn a_key_the_user_does_not_trust_is_forbidden_and_nothing_is_saved() {
    let fixture = Fixture::new();
    let prompt = Answers::new(false);

    let refused: CommandResult<_> = fixture.connect(Some(PASSWORD), &prompt);
    let error = refused.expect_err("refused");

    assert_eq!(error.code(), IpcErrorCode::Forbidden);
    assert_eq!(
        serialized(&error)["message"],
        "the server's host key was not trusted",
        "the text tells the user why"
    );
    assert!(
        !fixture.home.path().join(".ssh").join("known_hosts").exists(),
        "a no writes nothing"
    );
}

#[test]
fn disconnecting_closes_the_site_and_a_second_time_is_no_error() {
    let fixture = Fixture::new();
    let site = fixture.site();

    block_on(fixture.remotes.disconnect(site)).expect("closed");
    block_on(fixture.remotes.disconnect(site)).expect("closed again");

    assert!(fixture.remotes.pool().expect("a pool").fs(site).is_err());
}
