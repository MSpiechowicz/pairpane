//! Connections made as the real OpenSSH says (`ssh -G -F` a configuration of the test, aimed at the in-process server):
//! what is applied of its answer, and that a host it would send through a proxy is never connected to. Skipped, with a
//! note, on a machine with no OpenSSH client (see [`ssh_program`](super::ssh_program)).

use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::atomic::Ordering;

use pairpane_core::model::{ConnectOutcome, ConnectTarget};
use pretty_assertions::assert_eq;

use super::ssh_config::says;
use super::{Harness, assert_fixed, key_only};
use crate::AgentSource;
use crate::resolve::find_ssh;
use crate::test_server::{Settings, USER, generate_key, write_key};

pub(super) const PASSWORD: &str = "correct horse";

/// A path as `ssh_config` reads it on every system.
pub(super) fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// A harness whose `files` alias leads to the server, with `extra` lines in its block. Every file `ssh` would read by
/// default is named (the defaults are the real user's), unless `extra` names it.
pub(super) fn alias(settings: Settings, extra: &str) -> Option<Harness> {
    alias_agent(settings, AgentSource::None, extra)
}

/// The same, with an ssh-agent (the one that `IdentityAgent` of the configuration is not set to, or is `SSH_AUTH_SOCK`).
pub(super) fn alias_agent(settings: Settings, agent: AgentSource, extra: &str) -> Option<Harness> {
    Harness::via_ssh_agent(settings, agent, move |home, port| {
        let defaults = if extra.contains("UserKnownHostsFile") {
            String::new()
        } else {
            format!(
                "  UserKnownHostsFile {}\n",
                slashed(&home.join(".ssh").join("known_hosts"))
            )
        };

        format!(
            "Host files\n  HostName 127.0.0.1\n  Port {port}\n  User {USER}\n  GlobalKnownHostsFile {}\n{defaults}{extra}",
            slashed(&home.join("system_known_hosts"))
        )
        .replace("{home}", &slashed(home))
    })
}

pub(super) fn files_target() -> ConnectTarget {
    ConnectTarget {
        host: "files".to_owned(),
        port: None,
        user: None,
        key_file: None,
    }
}

/// Connects to the `files` alias of `h` with no secret, its server key trusted, and expects the refusal `kind` with
/// `message` (`what` says which case, for a failure). Gives the error.
pub(super) fn refused_with(h: &Harness, kind: ErrorKind, message: &str, what: &str) -> std::io::Error {
    h.trust();

    let error = h.connect_to(&files_target(), None).expect_err(what);

    assert_eq!(error.kind(), kind, "{what}");
    assert_eq!(error.to_string(), message, "{what}");

    error
}

/// Whether the `files` alias of `h` signs in with no secret, its server key trusted: no, when the server accepts no way
/// that was tried (anything else is a mistake of the test).
pub(super) fn signs_in(h: &Harness, what: &str) -> bool {
    h.trust();

    match h.connect_to(&files_target(), None) {
        Ok(ConnectOutcome::Connected { .. }) => true,
        Err(error) => {
            assert_eq!(error.kind(), ErrorKind::PermissionDenied, "{what}");
            assert_eq!(
                error.to_string(),
                "the server did not accept any way of signing in",
                "{what}"
            );

            false
        }
        Ok(other) => panic!("{what}: {other:?}"),
    }
}

pub(super) fn connected(h: &Harness) {
    match h.connect_to(&files_target(), Some(PASSWORD)) {
        Ok(ConnectOutcome::Connected { .. }) => {}
        Ok(other) => panic!("expected a connection, got {other:?}"),
        Err(error) => panic!(
            "not connected: {error}\n{}",
            says(find_ssh().as_deref(), Some(&h.home.path().join("ssh_config")), "files")
        ),
    }
}

#[test]
fn an_alias_connects_where_ssh_says_and_is_remembered_under_that_address() {
    let Some(h) = alias(Settings::default(), "") else {
        return;
    };
    let port = h.server.addr.port();

    match h.connect_to(&files_target(), Some(PASSWORD)).expect("answered") {
        ConnectOutcome::Connected { label, .. } => assert_eq!(label, format!("{USER}@files:{port}")),
        other => panic!("expected a connection, got {other:?}"),
    }

    let asked = h.prompt.asked.lock().expect("asked").clone();

    assert_eq!(asked.len(), 1);
    assert_eq!((asked[0].host.as_str(), asked[0].port), ("127.0.0.1", port));
    assert!(
        fs::read_to_string(h.known_hosts())
            .expect("file")
            .starts_with(&format!("[127.0.0.1]:{port} ssh-ed25519 "))
    );
}

#[test]
fn a_host_key_alias_is_the_name_the_key_is_looked_up_and_saved_under() {
    let Some(h) = alias(Settings::default(), "  HostKeyAlias Vault.Example\n") else {
        return;
    };

    connected(&h);

    let saved = fs::read_to_string(h.known_hosts()).expect("file");

    assert!(saved.starts_with("vault.example ssh-ed25519 "), "{saved}");
    assert_eq!(saved.lines().count(), 1);

    // Found again by the alias: no second question and no second line.
    h.pool
        .disconnect(match h.connect_to(&files_target(), None).expect("same site") {
            ConnectOutcome::Connected { site, .. } => site,
            other => panic!("expected a connection, got {other:?}"),
        });
    connected(&h);
    assert_eq!(h.prompt.times(), 1);
    assert_eq!(fs::read_to_string(h.known_hosts()).expect("file"), saved);
}

#[test]
fn the_known_hosts_files_of_ssh_are_read_and_the_first_is_written() {
    let Some(h) = alias(
        Settings::default(),
        "  UserKnownHostsFile {home}/first_known_hosts {home}/.ssh/known_hosts\n",
    ) else {
        return;
    };

    // The second file knows the key: no question, and nothing is written to the first.
    h.trust();
    connected(&h);
    assert_eq!(h.prompt.times(), 0);
    assert!(!h.home.path().join("first_known_hosts").exists());

    // A key that is in neither is asked about and goes to the first.
    let h = alias(
        Settings::default(),
        "  UserKnownHostsFile {home}/first_known_hosts {home}/.ssh/known_hosts\n",
    )
    .expect("ssh is there");

    connected(&h);
    assert_eq!(h.prompt.times(), 1);
    assert!(h.home.path().join("first_known_hosts").exists());
    assert!(!h.known_hosts().exists());
}

#[test]
fn strict_host_key_checking_refuses_a_key_that_is_not_known_without_asking() {
    let Some(h) = alias(Settings::default(), "  StrictHostKeyChecking yes\n") else {
        return;
    };
    let error = h.connect_to(&files_target(), Some(PASSWORD)).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::PermissionDenied);
    assert_eq!(
        error.to_string(),
        "ssh_config requires a known host key for this server (StrictHostKeyChecking yes); add it with ssh first"
    );
    assert_fixed(&error, h.server.addr);
    assert_eq!(h.prompt.times(), 0);
    assert!(!h.known_hosts().exists());

    // Known, it connects.
    h.trust();
    connected(&h);
    assert_eq!(h.prompt.times(), 0);
}

#[test]
fn any_other_strictness_asks_and_never_accepts_by_itself() {
    for extra in [
        "  StrictHostKeyChecking accept-new\n",
        "  StrictHostKeyChecking no\n",
        "  StrictHostKeyChecking off\n",
        "  StrictHostKeyChecking ask\n",
    ] {
        let Some(mut h) = alias(Settings::default(), extra) else {
            return;
        };

        h.prompt = super::Prompt::new(false);

        let error = h.connect_to(&files_target(), Some(PASSWORD)).expect_err("not trusted");

        assert_eq!(error.kind(), ErrorKind::PermissionDenied, "{extra}");
        assert_eq!(h.prompt.times(), 1, "{extra}");
        assert!(!h.known_hosts().exists(), "{extra}");
    }
}

#[test]
fn a_host_that_ssh_sends_through_a_proxy_is_never_connected_to() {
    for extra in ["  ProxyJump bastion\n", "  ProxyCommand nc %h %p\n"] {
        let Some(h) = alias(Settings::default(), extra) else {
            return;
        };
        let error = h.connect_to(&files_target(), None).expect_err("refused");

        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert_eq!(
            error.to_string(),
            "ssh_config sends this host through a proxy, which Pairpane does not support; connect with ssh"
        );
        assert_fixed(&error, h.server.addr);
        assert_eq!(h.server.counts.connections.load(Ordering::Relaxed), 0);
    }

    // `none` is no proxy.
    let Some(h) = alias(Settings::default(), "  ProxyJump none\n  ProxyCommand none\n") else {
        return;
    };

    h.trust();
    connected(&h);
}

#[test]
fn an_identity_file_of_the_configuration_signs_in() {
    let key = generate_key();
    let Some(h) = alias(key_only(&key), "  IdentityFile {home}/configured_key\n") else {
        return;
    };

    h.trust();
    write_key(&key, None, &h.home.path().join("configured_key"));

    assert!(matches!(
        h.connect_to(&files_target(), None).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));
}

#[test]
fn the_suggestions_are_the_plain_host_names_of_the_users_file_and_only_where_there_is_an_openssh() {
    let text = "Host hop\n  HostName 127.0.0.1\n  ProxyJump bastion\nHost named *.example\n  User someone\n";
    let Some(h) = alias(Settings::default(), "") else {
        return;
    };

    fs::write(h.ssh("config"), text).expect("config");

    assert_eq!(
        h.pool
            .hosts()
            .iter()
            .map(|host| host.alias.as_str())
            .collect::<Vec<_>>(),
        ["hop", "named"]
    );

    // Without OpenSSH the file cannot be read, so nothing in it is suggested.
    let bare = Harness::new(Settings::default());

    fs::write(bare.ssh("config"), text).expect("config");

    assert!(bare.pool.hosts().is_empty());
}
