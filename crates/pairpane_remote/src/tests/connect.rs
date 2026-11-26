//! Signing in: the methods, the secrets they ask for, and sharing a site.

use pairpane_core::model::{ConnectOutcome, SecretKind};
use pretty_assertions::assert_eq;

use super::{Harness, key_only, needed, path};
use crate::AgentSource;
use crate::test_server::{HOME, Settings, generate_key, write_key};

const PASSWORD: &str = "correct horse";

/// The wrong secret is refused as wrong (asking for `kind` again), and the right one connects.
fn wrong_then_right(h: &Harness, kind: SecretKind, wrong: &str, right: &str) {
    assert_eq!(
        h.answer(Some((kind, wrong))).expect("answered"),
        needed(kind, true, None)
    );
    assert!(matches!(
        h.answer(Some((kind, right))).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
}

#[test]
fn a_password_connects_and_the_site_starts_in_the_home_folder() {
    let h = Harness::new(Settings::default());

    h.trust();

    let ConnectOutcome::Connected { site, label, home } = h.connect(Some(PASSWORD)).expect("connected") else {
        panic!("not connected");
    };
    let port = h.server.addr.port();

    assert_eq!(label, format!("me@127.0.0.1:{port}"));
    assert_eq!(home, HOME);
    assert_eq!(h.pool.fs(site).expect("a site").label(), label);
    assert!(
        h.pool
            .fs(site)
            .expect("a site")
            .read_dir(&path(HOME), &Default::default())
            .expect("listed")
            .is_empty()
    );
}

#[test]
fn a_password_is_asked_for_and_asked_again_when_wrong() {
    let h = Harness::new(Settings::default());

    h.trust();

    assert_eq!(
        h.connect(None).expect("answered"),
        needed(SecretKind::Password, false, None)
    );
    wrong_then_right(&h, SecretKind::Password, "not it", PASSWORD);
}

#[test]
fn the_password_goes_as_the_answer_to_one_hidden_prompt_when_that_is_all_the_server_takes() {
    let h = Harness::new(Settings {
        keyboard_interactive: true,
        ..Settings::default()
    });

    h.trust();

    wrong_then_right(&h, SecretKind::Password, "not it", PASSWORD);
}

#[test]
fn a_default_key_signs_in_without_a_secret() {
    let key = generate_key();
    let h = Harness::new(key_only(&key));

    h.trust();
    write_key(&key, None, &h.ssh("id_ed25519"));

    assert!(matches!(
        h.connect(None).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
}

#[test]
fn a_named_key_is_tried() {
    let key = generate_key();
    let h = Harness::new(key_only(&key));

    h.trust();
    write_key(&key, None, &h.home.path().join("named_key"));

    let mut named = h.target();

    named.key_file = Some("~/named_key".to_owned());
    assert!(matches!(
        h.connect_to(&named, None).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
}

#[test]
fn an_encrypted_key_asks_for_its_passphrase_and_again_when_wrong() {
    let key = generate_key();
    let h = Harness::new(key_only(&key));

    h.trust();
    write_key(&key, Some("open sesame"), &h.ssh("id_ed25519"));

    assert_eq!(
        h.connect(None).expect("answered"),
        needed(SecretKind::Passphrase, false, None)
    );
    wrong_then_right(&h, SecretKind::Passphrase, "nope", "open sesame");
}

#[test]
fn a_server_that_takes_no_secret_we_have_is_refused_rather_than_asked() {
    let key = generate_key();
    let h = Harness::new(key_only(&key));

    h.trust();

    // Key sign-in only, and no key file: asking for a password would lead nowhere.
    let error = h.connect(None).expect_err("refused");

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
}

#[test]
fn the_same_server_is_one_site() {
    let h = Harness::new(Settings::default());

    h.trust();

    let first = h.site(Some(PASSWORD));
    let second = h.site(None);

    assert_eq!(first, second);
    assert_eq!(
        h.server.counts.connections.load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

/// An ssh-agent (an in-process server on a socket) that holds `key`; the runtime that serves it and the folder of its
/// socket live as long as the result.
#[cfg(unix)]
pub(super) fn agent_with(
    key: &russh::keys::PrivateKey,
) -> (tempfile::TempDir, std::path::PathBuf, tokio::runtime::Runtime) {
    use russh::keys::agent::client::AgentClient;
    use tokio_stream::wrappers::UnixListenerStream;

    let socket_dir = tempfile::tempdir().expect("temp dir");
    let socket = socket_dir.path().join("agent");
    let agent = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    let held = key.clone();
    let bound = socket.clone();

    agent.block_on(async move {
        let listener = tokio::net::UnixListener::bind(&bound).expect("socket");

        tokio::spawn(russh::keys::agent::server::serve(UnixListenerStream::new(listener), ()));

        let mut client = AgentClient::connect_uds(&bound).await.expect("agent client");

        client.add_identity(&held, &[]).await.expect("key added");
    });

    (socket_dir, socket, agent)
}

#[cfg(unix)]
#[test]
fn the_ssh_agent_signs_in() {
    let key = generate_key();
    let (_folder, socket, _agent) = agent_with(&key);
    let h = Harness::with(key_only(&key), super::quick(), AgentSource::Path(socket));

    h.trust();

    // No key file and no secret: only the agent can have signed in.
    assert!(matches!(
        h.connect(None).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
}

#[test]
fn a_missing_agent_does_not_stop_the_other_ways() {
    let h = Harness::with(
        Settings::default(),
        super::quick(),
        AgentSource::Path("/nonexistent/agent".into()),
    );

    h.trust();

    assert!(matches!(
        h.connect(Some(PASSWORD)).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
}
