//! What is done with the key a server presents.

use std::fs;
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;

use pairpane_core::model::ConnectOutcome;
use pretty_assertions::assert_eq;

use super::{Harness, Prompt, assert_fixed};
use crate::known_hosts::{KnownHosts, Name, Verdict};
use crate::test_server::{Killer, Settings, generate_key};
use crate::{HostKeyPrompt, UnknownHost};

const PASSWORD: &str = "correct horse";

fn verdict(h: &Harness) -> Verdict {
    let key = &h.server.host_key;

    KnownHosts::load(&h.known_hosts()).expect("readable").check(
        &Name::of("127.0.0.1", h.server.addr.port()).expect("a valid host"),
        key.algorithm().as_str(),
        &key.to_bytes().expect("key bytes"),
    )
}

#[test]
fn a_new_key_is_asked_about_once_and_a_yes_is_remembered() {
    let h = Harness::new(Settings::default());

    let site = h.site(Some(PASSWORD));

    h.pool.disconnect(site);

    let question = h.prompt.asked.lock().expect("asked").clone();

    assert_eq!(question.len(), 1);
    assert_eq!(question[0].host, "127.0.0.1");
    assert_eq!(question[0].port, h.server.addr.port());
    assert_eq!(question[0].algorithm, "ssh-ed25519");
    assert!(
        question[0].fingerprint.starts_with("SHA256:"),
        "{}",
        question[0].fingerprint
    );
    assert_eq!(verdict(&h), Verdict::Known);

    // The next connection to the server finds the key in the file.
    h.site(Some(PASSWORD));
    assert_eq!(h.prompt.times(), 1);
    assert_eq!(fs::read_to_string(h.known_hosts()).expect("file").lines().count(), 1);
}

#[test]
fn a_no_connects_nothing_and_writes_nothing() {
    let mut h = Harness::new(Settings::default());

    h.prompt = Prompt::new(false);

    let error = h.connect(Some(PASSWORD)).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::PermissionDenied);
    assert_fixed(&error, h.server.addr);
    assert_eq!(h.prompt.times(), 1);
    assert!(!h.known_hosts().exists());
}

#[test]
fn a_changed_key_is_refused_without_a_question_and_the_file_is_left_alone() {
    let h = Harness::new(Settings::default());
    let other = generate_key();
    let port = h.server.addr.port();
    let old = format!("# mine\n[127.0.0.1]:{port} ssh-ed25519 {}\n", {
        use base64::Engine;

        base64::engine::general_purpose::STANDARD.encode(other.public_key().to_bytes().expect("bytes"))
    });

    fs::write(h.known_hosts(), &old).expect("written");

    let error = h.connect(Some(PASSWORD)).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::PermissionDenied);
    assert!(error.to_string().contains("line 2"), "{error}");
    assert_fixed(&error, h.server.addr);
    assert_eq!(h.prompt.times(), 0);
    assert_eq!(fs::read_to_string(h.known_hosts()).expect("file"), old);
}

#[test]
fn a_revoked_key_is_refused_too() {
    let h = Harness::new(Settings::default());
    h.trust();

    let trusted = fs::read_to_string(h.known_hosts()).expect("file");
    let blob = trusted.split_whitespace().nth(2).expect("a key").to_owned();

    fs::write(h.known_hosts(), format!("{trusted}@revoked * ssh-ed25519 {blob}\n")).expect("written");

    let error = h.connect(Some(PASSWORD)).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::PermissionDenied);
    assert!(error.to_string().contains("line 2"), "{error}");
}

#[test]
fn a_key_that_is_trusted_is_not_asked_about() {
    let h = Harness::new(Settings::default());

    h.trust();

    assert!(matches!(
        h.connect(Some(PASSWORD)).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
    assert_eq!(h.prompt.times(), 0);
}

fn blob_of(h: &Harness) -> String {
    use base64::Engine;

    base64::engine::general_purpose::STANDARD.encode(h.server.host_key.to_bytes().expect("key bytes"))
}

#[test]
fn a_key_the_system_file_holds_is_trusted_and_that_file_is_never_written() {
    let h = Harness::new(Settings::default());
    let line = format!("[127.0.0.1]:{} ssh-ed25519 {}\n", h.server.addr.port(), blob_of(&h));

    fs::write(h.system_known_hosts(), &line).expect("written");

    assert!(matches!(
        h.connect(Some(PASSWORD)).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
    assert_eq!(h.prompt.times(), 0);
    assert!(!h.known_hosts().exists());
    assert_eq!(fs::read_to_string(h.system_known_hosts()).expect("file"), line);
}

#[test]
fn a_key_the_system_file_revokes_or_changes_is_refused_without_a_question() {
    let h = Harness::new(Settings::default());
    let port = h.server.addr.port();
    let other = generate_key();
    let changed = {
        use base64::Engine;

        base64::engine::general_purpose::STANDARD.encode(other.public_key().to_bytes().expect("bytes"))
    };

    for line in [
        format!("@revoked * ssh-ed25519 {}\n", blob_of(&h)),
        format!("[127.0.0.1]:{port} ssh-ed25519 {changed}\n"),
    ] {
        fs::write(h.system_known_hosts(), &line).expect("written");

        let error = h.connect(Some(PASSWORD)).expect_err("refused");

        assert_eq!(error.kind(), ErrorKind::PermissionDenied, "{line}");
        assert!(error.to_string().contains("another known_hosts file"), "{error}");
        assert_fixed(&error, h.server.addr);
    }

    assert_eq!(h.prompt.times(), 0);
    assert!(!h.known_hosts().exists());
}

#[test]
fn a_server_that_hangs_up_while_the_user_is_asked_says_so() {
    struct Hangs(Killer);

    impl HostKeyPrompt for Hangs {
        fn trust_new(&self, _: &UnknownHost) -> bool {
            self.0.kill();
            std::thread::sleep(Duration::from_millis(100));

            true
        }
    }

    let h = Harness::new(Settings::default());
    let error = h
        .pool
        .connect(&h.target(), None, Arc::new(Hangs(h.server.killer())))
        .expect_err("closed");

    assert_eq!(error.kind(), ErrorKind::ConnectionAborted);
    assert_eq!(
        error.to_string(),
        "the server closed the connection while waiting for an answer; connect again"
    );
}

const UNUSABLE: &str = "known_hosts vouches for this server in a form Pairpane cannot check (a certificate authority or an unsupported key type); connect with ssh first";

#[test]
fn a_host_that_known_hosts_vouches_for_in_a_form_that_cannot_be_checked_is_refused_without_a_question() {
    let h = Harness::new(Settings::default());
    let name = format!("[127.0.0.1]:{}", h.server.addr.port());
    let blob = blob_of(&h);

    for (file, line) in [
        (h.known_hosts(), format!("@cert-authority {name} ssh-ed25519 {blob}\n")),
        (h.known_hosts(), format!("{name} ssh-dss {blob}\n")),
        (h.known_hosts(), format!("{name} sk-ssh-ed25519@openssh.com {blob}\n")),
        (
            h.system_known_hosts(),
            format!("@cert-authority * ssh-ed25519 {blob}\n"),
        ),
    ] {
        fs::write(&file, &line).expect("written");

        let error = h.connect(Some(PASSWORD)).expect_err("refused");

        assert_eq!(error.kind(), ErrorKind::PermissionDenied, "{line}");
        assert_eq!(error.to_string(), UNUSABLE, "{line}");
        assert_fixed(&error, h.server.addr);
        assert_eq!(fs::read_to_string(&file).expect("file"), line, "left alone");

        fs::remove_file(&file).expect("removed");
    }

    assert_eq!(h.prompt.times(), 0);
}

#[test]
fn a_key_that_is_trusted_is_still_known_next_to_a_line_that_cannot_be_checked() {
    let h = Harness::new(Settings::default());
    let name = format!("[127.0.0.1]:{}", h.server.addr.port());

    h.trust();

    let trusted = fs::read_to_string(h.known_hosts()).expect("file");

    fs::write(
        h.known_hosts(),
        format!("{name} ssh-dss AAAA\n@cert-authority {name} ssh-ed25519 AAAA\n{trusted}"),
    )
    .expect("written");

    assert!(matches!(
        h.connect(Some(PASSWORD)).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
    assert_eq!(h.prompt.times(), 0);
}

/// A `~/.ssh` that cannot be written to (as for a user whose files are read-only).
#[cfg(unix)]
struct ReadOnly(std::path::PathBuf);

#[cfg(unix)]
impl ReadOnly {
    fn new(folder: std::path::PathBuf) -> Self {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(&folder, fs::Permissions::from_mode(0o500)).expect("read-only");

        Self(folder)
    }
}

#[cfg(unix)]
impl Drop for ReadOnly {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;

        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
    }
}

#[cfg(unix)]
#[test]
fn a_yes_that_could_not_be_saved_is_remembered_for_the_next_step_of_the_sign_in() {
    use pairpane_core::model::SecretKind;

    let h = Harness::new(Settings::default());
    let _read_only = ReadOnly::new(h.home.path().join(".ssh"));

    // The first connection asks about the key and then for the password; the one with the password does not ask again.
    assert_eq!(
        h.connect(None).expect("answered"),
        super::needed(SecretKind::Password, false, None)
    );
    assert!(matches!(
        h.connect(Some(PASSWORD)).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
    assert_eq!(h.prompt.times(), 1);
    assert!(!h.known_hosts().exists(), "it could not be saved");
}
