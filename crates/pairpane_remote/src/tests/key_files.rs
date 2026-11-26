//! Key files that are not what they should be: a pipe is not waited for, a link to a key is a key.

use std::io::ErrorKind;
use std::time::{Duration, Instant};

use pairpane_core::model::ConnectOutcome;
use pretty_assertions::assert_eq;

use super::{Harness, key_only};
use crate::test_server::{Settings, generate_key, write_key};

const PASSWORD: &str = "correct horse";

fn fifo(at: &std::path::Path) {
    let made = std::process::Command::new("mkfifo")
        .arg(at)
        .status()
        .expect("mkfifo runs");

    assert!(made.success());
}

#[test]
fn a_pipe_as_a_default_key_is_skipped_without_waiting() {
    let h = Harness::new(Settings {
        authorized_key: Some(generate_key().public_key().clone()),
        ..Settings::default()
    });

    h.trust();
    fifo(&h.ssh("id_ed25519"));

    let started = Instant::now();

    assert!(matches!(
        h.connect(Some(PASSWORD)).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
}

#[test]
fn a_pipe_as_the_named_key_is_refused_without_waiting() {
    // Key sign-in is offered, so the key files are looked at.
    let h = Harness::new(Settings {
        authorized_key: Some(generate_key().public_key().clone()),
        ..Settings::default()
    });
    let mut target = h.target();

    h.trust();
    fifo(&h.home.path().join("pipe"));
    target.key_file = Some("~/pipe".to_owned());

    let started = Instant::now();
    let error = h.connect_to(&target, Some(PASSWORD)).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "the key file is not a regular file");
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
}

#[test]
fn a_link_to_a_key_file_is_a_key_file() {
    let key = generate_key();
    let h = Harness::new(key_only(&key));

    h.trust();
    write_key(&key, None, &h.home.path().join("real_key"));
    std::os::unix::fs::symlink(h.home.path().join("real_key"), h.ssh("id_ed25519")).expect("link");

    assert!(matches!(
        h.connect(None).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
}
