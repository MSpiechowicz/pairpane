//! `BatchMode` and `NumberOfPasswordPrompts` of the configuration: where `ssh` would not ask, Pairpane does not either (a
//! new host key under `BatchMode`, a password or a passphrase under either), and says so in a fixed message. Skipped, with
//! a note, on a machine with no OpenSSH client (see [`ssh_program`](super::ssh_program)).

use std::io::ErrorKind;

use pairpane_core::model::SecretKind;
use pretty_assertions::assert_eq;

use super::ssh_connect::{alias, files_target, refused_with, signs_in};
use super::{key_only, needed};
use crate::test_server::{Settings, generate_key, write_key};

const BATCH_KEY: &str =
    "ssh_config sets BatchMode, so Pairpane will not ask about a new host key; add it with ssh first";

const NO_QUESTION: &str = "ssh_config forbids asking for a password or a passphrase (BatchMode, or NumberOfPasswordPrompts 0); use a key that needs none, or connect with ssh";

#[test]
fn batch_mode_refuses_a_new_host_key_without_asking_and_keeps_a_known_one() {
    let Some(h) = alias(Settings::default(), "  BatchMode yes\n") else {
        return;
    };
    let error = h.connect_to(&files_target(), None).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::PermissionDenied);
    assert_eq!(error.to_string(), BATCH_KEY);
    assert_eq!(h.prompt.times(), 0);
    assert!(!h.known_hosts().exists());

    // Known, the key is no question: and the password that would be asked for is.
    refused_with(&h, ErrorKind::PermissionDenied, NO_QUESTION, "a known key");
}

#[test]
fn no_password_prompts_means_no_question_for_a_password_or_a_passphrase() {
    let key = generate_key();

    for extra in ["  NumberOfPasswordPrompts 0\n", "  BatchMode yes\n"] {
        // A password on offer.
        let Some(h) = alias(Settings::default(), extra) else {
            return;
        };

        refused_with(&h, ErrorKind::PermissionDenied, NO_QUESTION, extra);

        // A key that is locked, and no password.
        let Some(h) = alias(key_only(&key), extra) else {
            return;
        };

        write_key(&key, Some("open sesame"), &h.ssh("id_ed25519"));
        refused_with(&h, ErrorKind::PermissionDenied, NO_QUESTION, extra);
    }

    // A new host key is a question of its own, and `NumberOfPasswordPrompts` does not forbid it.
    let Some(h) = alias(Settings::default(), "  NumberOfPasswordPrompts 0\n") else {
        return;
    };

    assert_eq!(
        h.connect_to(&files_target(), None).expect_err("refused").to_string(),
        NO_QUESTION
    );
    assert_eq!(h.prompt.times(), 1);

    // Three is what `ssh` asks by default: asked as ever.
    let Some(h) = alias(Settings::default(), "  NumberOfPasswordPrompts 3\n") else {
        return;
    };

    h.trust();
    assert_eq!(
        h.connect_to(&files_target(), None).expect("answered"),
        needed(SecretKind::Password, false, None)
    );
}

#[test]
fn a_key_that_needs_no_secret_still_signs_in_under_batch_mode() {
    let key = generate_key();
    let Some(h) = alias(key_only(&key), "  BatchMode yes\n  IdentityFile {home}/k\n") else {
        return;
    };

    write_key(&key, None, &h.home.path().join("k"));
    assert!(signs_in(&h, "a key without a passphrase"));
}
